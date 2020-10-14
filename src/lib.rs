//! A lock-free concurrent slab.
//!
//! Slabs provide pre-allocated storage for many instances of a single data
//! type. When a large number of values of a single type are required,
//! this can be more efficient than allocating each item individually. Since the
//! allocated items are the same size, memory fragmentation is reduced, and
//! creating and removing new items can be very cheap.
//!
//! This crate implements a lock-free concurrent slab, indexed by `usize`s.
//!
//! ## Usage
//!
//! First, add this to your `Cargo.toml`:
//!
//! ```toml
//! sharded-slab = "0.0.9"
//! ```
//!
//! This crate provides two  types, [`Slab`] and [`Pool`], which provide
//! slightly different APIs for using a sharded slab.
//!
//! [`Slab`] implements a slab for _storing_ small types, sharing them between
//! threads, and accessing them by index. New entries are allocated by
//! [inserting] data, moving it in by value. Similarly, entries may be
//! deallocated by [taking] from the slab, moving the value out. This API is
//! similar to a `Vec<Option<T>>`, but allowing lock-free concurrent insertion
//! and removal.
//!
//! In contrast, the [`Pool`] type provides an [object pool] style API for
//! _reusing storage_. Rather than constructing values and moving them into the
//! pool, as with [`Slab`], [allocating an entry][create] from the pool takes a
//! closure that's provided with a mutable reference to initialize the entry in
//! place. When entries are deallocated, they are [cleared] in place. Types
//! which own a heap allocation can be cleared by dropping any _data_ they
//! store, but retaining any previously-allocated capacity. This means that a
//! [`Pool`] may be used to reuse a set of existing heap allocations, reducing
//! allocator load.
//!
//! [`Slab`]: struct.Slab.html
//! [inserting]: struct.Slab.html#method.insert
//! [taking]: struct.Slab.html#method.take
//! [`Pool`]: struct.Pool.html
//! [create]: struct.Pool.html#method.create
//! [cleared]: trait.Clear.html
//! [object pool]: https://en.wikipedia.org/wiki/Object_pool_pattern
//!
//! # Examples
//!
//! Inserting an item into the slab, returning an index:
//! ```rust
//! # use sharded_slab::Slab;
//! let slab = Slab::new();
//!
//! let key = slab.insert("hello world").unwrap();
//! assert_eq!(slab.get(key).unwrap(), "hello world");
//! ```
//!
//! To share a slab across threads, it may be wrapped in an `Arc`:
//! ```rust
//! # use sharded_slab::Slab;
//! use std::sync::Arc;
//! let slab = Arc::new(Slab::new());
//!
//! let slab2 = slab.clone();
//! let thread2 = std::thread::spawn(move || {
//!     let key = slab2.insert("hello from thread two").unwrap();
//!     assert_eq!(slab2.get(key).unwrap(), "hello from thread two");
//!     key
//! });
//!
//! let key1 = slab.insert("hello from thread one").unwrap();
//! assert_eq!(slab.get(key1).unwrap(), "hello from thread one");
//!
//! // Wait for thread 2 to complete.
//! let key2 = thread2.join().unwrap();
//!
//! // The item inserted by thread 2 remains in the slab.
//! assert_eq!(slab.get(key2).unwrap(), "hello from thread two");
//!```
//!
//! If items in the slab must be mutated, a `Mutex` or `RwLock` may be used for
//! each item, providing granular locking of items rather than of the slab:
//!
//! ```rust
//! # use sharded_slab::Slab;
//! use std::sync::{Arc, Mutex};
//! let slab = Arc::new(Slab::new());
//!
//! let key = slab.insert(Mutex::new(String::from("hello world"))).unwrap();
//!
//! let slab2 = slab.clone();
//! let thread2 = std::thread::spawn(move || {
//!     let hello = slab2.get(key).expect("item missing");
//!     let mut hello = hello.lock().expect("mutex poisoned");
//!     *hello = String::from("hello everyone!");
//! });
//!
//! thread2.join().unwrap();
//!
//! let hello = slab.get(key).expect("item missing");
//! let mut hello = hello.lock().expect("mutex poisoned");
//! assert_eq!(hello.as_str(), "hello everyone!");
//! ```
//!
//! # Configuration
//!
//! For performance reasons, several values used by the slab are calculated as
//! constants. In order to allow users to tune the slab's parameters, we provide
//! a [`Config`] trait which defines these parameters as associated `consts`.
//! The `Slab` type is generic over a `C: Config` parameter.
//!
//! [`Config`]: trait.Config.html
//!
//! # Comparison with Similar Crates
//!
//! - [`slab`]: Carl Lerche's `slab` crate provides a slab implementation with a
//!   similar API, implemented by storing all data in a single vector.
//!
//!   Unlike `sharded_slab`, inserting and removing elements from the slab
//!   requires  mutable access. This means that if the slab is accessed
//!   concurrently by multiple threads, it is necessary for it to be protected
//!   by a `Mutex` or `RwLock`. Items may not be inserted or removed (or
//!   accessed, if a `Mutex` is used) concurrently, even when they are
//!   unrelated. In many cases, the lock can become a significant bottleneck. On
//!   the other hand, this crate allows separate indices in the slab to be
//!   accessed, inserted, and removed concurrently without requiring a global
//!   lock. Therefore, when the slab is shared across multiple threads, this
//!   crate offers significantly better performance than `slab`.
//!
//!   However, the lock free slab introduces some additional constant-factor
//!   overhead. This means that in use-cases where a slab is _not_ shared by
//!   multiple threads and locking is not required, this crate will likely offer
//!   slightly worse performance.
//!
//!   In summary: `sharded-slab` offers significantly improved performance in
//!   concurrent use-cases, while `slab` should be preferred in single-threaded
//!   use-cases.
//!
//! [`slab`]: https://crates.io/crates/loom
//!
//! # Safety and Correctness
//!
//! Most implementations of lock-free data structures in Rust require some
//! amount of unsafe code, and this crate is not an exception. In order to catch
//! potential bugs in this unsafe code, we make use of [`loom`], a
//! permutation-testing tool for concurrent Rust programs. All `unsafe` blocks
//! this crate occur in accesses to `loom` `UnsafeCell`s. This means that when
//! those accesses occur in this crate's tests, `loom` will assert that they are
//! valid under the C11 memory model across multiple permutations of concurrent
//! executions of those tests.
//!
//! In order to guard against the [ABA problem][aba], this crate makes use of
//! _generational indices_. Each slot in the slab tracks a generation counter
//! which is incremented every time a value is inserted into that slot, and the
//! indices returned by [`Slab::insert`] include the generation of the slot when
//! the value was inserted, packed into the high-order bits of the index. This
//! ensures that if a value is inserted, removed,  and a new value is inserted
//! into the same slot in the slab, the key returned by the first call to
//! `insert` will not map to the new value.
//!
//! Since a fixed number of bits are set aside to use for storing the generation
//! counter, the counter will wrap  around after being incremented a number of
//! times. To avoid situations where a returned index lives long enough to see the
//! generation counter wrap around to the same value, it is good to be fairly
//! generous when configuring the allocation of index bits.
//!
//! [`loom`]: https://crates.io/crates/loom
//! [aba]: https://en.wikipedia.org/wiki/ABA_problem
//! [`Slab::insert`]: struct.Slab.html#method.insert
//!
//! # Performance
//!
//! These graphs were produced by [benchmarks] of the sharded slab implementation,
//! using the [`criterion`] crate.
//!
//! The first shows the results of a benchmark where an increasing number of
//! items are inserted and then removed into a slab concurrently by five
//! threads. It compares the performance of the sharded slab implementation
//! with a `RwLock<slab::Slab>`:
//!
//! <img width="1124" alt="Screen Shot 2019-10-01 at 5 09 49 PM" src="https://user-images.githubusercontent.com/2796466/66078398-cd6c9f80-e516-11e9-9923-0ed6292e8498.png">
//!
//! The second graph shows the results of a benchmark where an increasing
//! number of items are inserted and then removed by a _single_ thread. It
//! compares the performance of the sharded slab implementation with an
//! `RwLock<slab::Slab>` and a `mut slab::Slab`.
//!
//! <img width="925" alt="Screen Shot 2019-10-01 at 5 13 45 PM" src="https://user-images.githubusercontent.com/2796466/66078469-f0974f00-e516-11e9-95b5-f65f0aa7e494.png">
//!
//! These benchmarks demonstrate that, while the sharded approach introduces
//! a small constant-factor overhead, it offers significantly better
//! performance across concurrent accesses.
//!
//! [benchmarks]: https://github.com/hawkw/sharded-slab/blob/master/benches/bench.rs
//! [`criterion`]: https://crates.io/crates/criterion
//!
//! # Implementation Notes
//!
//! See [this page](implementation/index.html) for details on this crate's design
//! and implementation.
//!
#![doc(html_root_url = "https://docs.rs/sharded-slab/0.0.3")]

#[cfg(test)]
macro_rules! thread_local {
    ($($tts:tt)+) => { loom::thread_local!{ $($tts)+ } }
}

#[cfg(not(test))]
macro_rules! thread_local {
    ($($tts:tt)+) => { std::thread_local!{ $($tts)+ } }
}

macro_rules! test_println {
    ($($arg:tt)*) => {
        if cfg!(test) && cfg!(slab_print) {
            println!("[{:?} {}:{}] {}", crate::Tid::<crate::DefaultConfig>::current(), file!(), line!(), format_args!($($arg)*))
        }
    }
}

mod clear;
pub mod implementation;
mod page;
mod pool;
pub(crate) mod sync;
mod tid;
pub(crate) use tid::Tid;
pub(crate) mod cfg;
mod iter;
mod shard;
use cfg::CfgPrivate;
pub use cfg::{Config, DefaultConfig};
pub use clear::Clear;
pub use pool::{Pool, PoolGuard};

use shard::Shard;
use std::{fmt, marker::PhantomData};

/// A sharded slab.
///
/// See the [crate-level documentation](index.html) for details on using this type.
pub struct Slab<T, C: cfg::Config = DefaultConfig> {
    shards: Box<[Shard<Option<T>, C>]>,
    _cfg: PhantomData<C>,
}

/// A guard that allows access to an object in a slab.
///
/// While the guard exists, it indicates to the slab that the item the guard
/// references is currently being accessed. If the item is removed from the slab
/// while a guard exists, the removal will be deferred until all guards are dropped.
pub struct Guard<'a, T, C: cfg::Config = DefaultConfig> {
    pub(crate) inner: page::slot::Guard<'a, T, C>,
    pub(crate) shard: &'a Shard<Option<T>, C>,
    pub(crate) key: usize,
}

impl<T> Slab<T> {
    /// Returns a new slab with the default configuration parameters.
    pub fn new() -> Self {
        Self::new_with_config()
    }

    /// Returns a new slab with the provided configuration parameters.
    pub fn new_with_config<C: cfg::Config>() -> Slab<T, C> {
        C::validate();
        let shards = (0..C::MAX_SHARDS).map(Shard::new).collect();
        Slab {
            shards,
            _cfg: PhantomData,
        }
    }
}

impl<T, C: cfg::Config> Slab<T, C> {
    /// The number of bits in each index which are used by the slab.
    ///
    /// If other data is packed into the `usize` indices returned by
    /// [`Slab::insert`], user code is free to use any bits higher than the
    /// `USED_BITS`-th bit freely.
    ///
    /// This is determined by the [`Config`] type that configures the slab's
    /// parameters. By default, all bits are used; this can be changed by
    /// overriding the [`Config::RESERVED_BITS`][res] constant.
    ///
    /// [`Config`]: trait.Config.html
    /// [res]: trait.Config.html#associatedconstant.RESERVED_BITS
    /// [`Slab::insert`]: struct.Slab.html#method.insert
    pub const USED_BITS: usize = C::USED_BITS;

    /// Inserts a value into the slab, returning a key that can be used to
    /// access it.
    ///
    /// If this function returns `None`, then the shard for the current thread
    /// is full and no items can be added until some are removed, or the maximum
    /// number of shards has been reached.
    ///
    /// # Examples
    /// ```rust
    /// # use sharded_slab::Slab;
    /// let slab = Slab::new();
    ///
    /// let key = slab.insert("hello world").unwrap();
    /// assert_eq!(slab.get(key).unwrap(), "hello world");
    /// ```
    pub fn insert(&self, value: T) -> Option<usize> {
        let tid = Tid::<C>::current();
        test_println!("insert {:?}", tid);
        let mut value = Some(value);
        self.shards[tid.as_usize()]
            .init_with(|slot| slot.insert(&mut value))
            .map(|idx| tid.pack(idx))
    }

    /// Remove the value associated with the given key from the slab, returning
    /// `true` if a value was removed.
    ///
    /// Unlike [`take`], this method does _not_ block the current thread until
    /// the value can be removed. Instead, if another thread is currently
    /// accessing that value, this marks it to be removed by that thread when it
    /// finishes accessing the value.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let slab = sharded_slab::Slab::new();
    /// let key = slab.insert("hello world").unwrap();
    ///
    /// // Remove the item from the slab.
    /// assert!(slab.remove(key));
    ///
    /// // Now, the slot is empty.
    /// assert!(!slab.contains(key));
    /// ```
    ///
    /// ```rust
    /// use std::sync::Arc;
    ///
    /// let slab = Arc::new(sharded_slab::Slab::new());
    /// let key = slab.insert("hello world").unwrap();
    ///
    /// let slab2 = slab.clone();
    /// let thread2 = std::thread::spawn(move || {
    ///     // Depending on when this thread begins executing, the item may
    ///     // or may not have already been removed...
    ///     if let Some(item) = slab2.get(key) {
    ///         assert_eq!(item, "hello world");
    ///     }
    /// });
    ///
    /// // The item will be removed by thread2 when it finishes accessing it.
    /// assert!(slab.remove(key));
    ///
    /// thread2.join().unwrap();
    /// assert!(!slab.contains(key));
    /// ```
    /// [`take`]: #method.take
    pub fn remove(&self, idx: usize) -> bool {
        // The `Drop` impl for `Guard` calls `remove_local` or `remove_remote` based
        // on where the guard was dropped from. If the dropped guard was the last one, this will
        // call `Slot::remove_value` which actually clears storage.
        let tid = C::unpack_tid(idx);

        test_println!("rm_deferred {:?}", tid);
        let shard = self.shards.get(tid.as_usize());
        if tid.is_current() {
            shard.map(|shard| shard.remove_local(idx)).unwrap_or(false)
        } else {
            shard.map(|shard| shard.remove_remote(idx)).unwrap_or(false)
        }
    }

    /// Removes the value associated with the given key from the slab, returning
    /// it.
    ///
    /// If the slab does not contain a value for that key, `None` is returned
    /// instead.
    ///
    /// If the value associated with the given key is currently being
    /// accessed by another thread, this method will block the current thread
    /// until the item is no longer accessed. If this is not desired, use
    /// [`remove`] instead.
    ///
    /// **Note**: This method blocks the calling thread by spinning until the
    /// currently outstanding references are released. Spinning for long periods
    /// of time can result in high CPU time and power consumption. Therefore,
    /// `take` should only be called when other references to the slot are
    /// expected to be dropped soon (e.g., when all accesses are relatively
    /// short).
    ///
    /// # Examples
    ///
    /// ```rust
    /// let slab = sharded_slab::Slab::new();
    /// let key = slab.insert("hello world").unwrap();
    ///
    /// // Remove the item from the slab, returning it.
    /// assert_eq!(slab.take(key), Some("hello world"));
    ///
    /// // Now, the slot is empty.
    /// assert!(!slab.contains(key));
    /// ```
    ///
    /// ```rust
    /// use std::sync::Arc;
    ///
    /// let slab = Arc::new(sharded_slab::Slab::new());
    /// let key = slab.insert("hello world").unwrap();
    ///
    /// let slab2 = slab.clone();
    /// let thread2 = std::thread::spawn(move || {
    ///     // Depending on when this thread begins executing, the item may
    ///     // or may not have already been removed...
    ///     if let Some(item) = slab2.get(key) {
    ///         assert_eq!(item, "hello world");
    ///     }
    /// });
    ///
    /// // The item will only be removed when the other thread finishes
    /// // accessing it.
    /// assert_eq!(slab.take(key), Some("hello world"));
    ///
    /// thread2.join().unwrap();
    /// assert!(!slab.contains(key));
    /// ```
    /// [`remove`]: #method.remove
    pub fn take(&self, idx: usize) -> Option<T> {
        let tid = C::unpack_tid(idx);

        test_println!("rm {:?}", tid);
        let shard = &self.shards[tid.as_usize()];
        if tid.is_current() {
            shard.take_local(idx)
        } else {
            shard.take_remote(idx)
        }
    }

    /// Return a reference to the value associated with the given key.
    ///
    /// If the slab does not contain a value for the given key, or if the
    /// maximum number of concurrent references to the slot has been reached,
    /// `None` is returned instead.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let slab = sharded_slab::Slab::new();
    /// let key = slab.insert("hello world").unwrap();
    ///
    /// assert_eq!(slab.get(key).unwrap(), "hello world");
    /// assert!(slab.get(12345).is_none());
    /// ```
    pub fn get(&self, key: usize) -> Option<Guard<'_, T, C>> {
        let tid = C::unpack_tid(key);

        test_println!("get {:?}; current={:?}", tid, Tid::<C>::current());
        let shard = self.shards.get(tid.as_usize())?;
        let inner = shard.get(key, |x| {
            x.as_ref().expect(
                "if a slot can be accessed at the current generation, its value must be `Some`",
            )
        })?;

        Some(Guard { inner, shard, key })
    }

    /// Returns `true` if the slab contains a value for the given key.
    ///
    /// # Examples
    ///
    /// ```
    /// let slab = sharded_slab::Slab::new();
    ///
    /// let key = slab.insert("hello world").unwrap();
    /// assert!(slab.contains(key));
    ///
    /// slab.take(key).unwrap();
    /// assert!(!slab.contains(key));
    /// ```
    pub fn contains(&self, key: usize) -> bool {
        self.get(key).is_some()
    }

    /// Returns an iterator over all the items in the slab.
    pub fn unique_iter(&mut self) -> iter::UniqueIter<'_, T, C> {
        let mut shards = self.shards.iter_mut();
        let shard = shards.next().expect("must be at least 1 shard");
        let mut pages = shard.iter();
        let slots = pages.next().and_then(page::Shared::iter_unique);
        iter::UniqueIter {
            shards,
            slots,
            pages,
        }
    }

    pub fn iter(&self) -> iter::Iter<'_, T, C> {
        let mut shards = self.shards.iter();
        let current_shard = shards.next().expect("must be at least 1 shard");
        let mut pages = current_shard.iter();
        let page = pages.next();
        let current_page_sz = page.map(page::Shared::prev_sz).unwrap_or(0);
        let slots = page.and_then(page::Shared::iter);
        iter::Iter {
            shards,
            current_shard,
            current_page_sz,
            slots,
            pages,
        }
    }
}

impl<T> Default for Slab<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: fmt::Debug, C: cfg::Config> fmt::Debug for Slab<T, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Slab")
            .field("shards", &self.shards)
            .field("config", &C::debug())
            .finish()
    }
}

unsafe impl<T: Send, C: cfg::Config> Send for Slab<T, C> {}
unsafe impl<T: Sync, C: cfg::Config> Sync for Slab<T, C> {}

// === impl Guard ===

impl<'a, T, C: cfg::Config> Guard<'a, T, C> {
    /// Returns the key used to access the guard.
    pub fn key(&self) -> usize {
        self.key
    }
}

impl<'a, T, C: cfg::Config> std::ops::Deref for Guard<'a, T, C> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.inner.item()
    }
}

impl<'a, T, C: cfg::Config> Drop for Guard<'a, T, C> {
    fn drop(&mut self) {
        use crate::sync::atomic;
        if self.inner.release() {
            atomic::fence(atomic::Ordering::Acquire);
            if Tid::<C>::current().as_usize() == self.shard.tid {
                self.shard.remove_local(self.key);
            } else {
                self.shard.remove_remote(self.key);
            }
        }
    }
}

impl<'a, T, C> fmt::Debug for Guard<'a, T, C>
where
    T: fmt::Debug,
    C: cfg::Config,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.inner.item(), f)
    }
}

impl<'a, T, C> PartialEq<T> for Guard<'a, T, C>
where
    T: PartialEq<T>,
    C: cfg::Config,
{
    fn eq(&self, other: &T) -> bool {
        self.inner.item().eq(other)
    }
}

// === pack ===

pub(crate) trait Pack<C: cfg::Config>: Sized {
    // ====== provided by each implementation =================================

    /// The number of bits occupied by this type when packed into a usize.
    ///
    /// This must be provided to determine the number of bits into which to pack
    /// the type.
    const LEN: usize;
    /// The type packed on the less significant side of this type.
    ///
    /// If this type is packed into the least significant bit of a usize, this
    /// should be `()`, which occupies no bytes.
    ///
    /// This is used to calculate the shift amount for packing this value.
    type Prev: Pack<C>;

    // ====== calculated automatically ========================================

    /// A number consisting of `Self::LEN` 1 bits, starting at the least
    /// significant bit.
    ///
    /// This is the higest value this type can represent. This number is shifted
    /// left by `Self::SHIFT` bits to calculate this type's `MASK`.
    ///
    /// This is computed automatically based on `Self::LEN`.
    const BITS: usize = {
        let shift = 1 << (Self::LEN - 1);
        shift | (shift - 1)
    };
    /// The number of bits to shift a number to pack it into a usize with other
    /// values.
    ///
    /// This is caculated automatically based on the `LEN` and `SHIFT` constants
    /// of the previous value.
    const SHIFT: usize = Self::Prev::SHIFT + Self::Prev::LEN;

    /// The mask to extract only this type from a packed `usize`.
    ///
    /// This is calculated by shifting `Self::BITS` left by `Self::SHIFT`.
    const MASK: usize = Self::BITS << Self::SHIFT;

    fn as_usize(&self) -> usize;
    fn from_usize(val: usize) -> Self;

    #[inline(always)]
    fn pack(&self, to: usize) -> usize {
        let value = self.as_usize();
        debug_assert!(value <= Self::BITS);

        (to & !Self::MASK) | (value << Self::SHIFT)
    }

    #[inline(always)]
    fn from_packed(from: usize) -> Self {
        let value = (from & Self::MASK) >> Self::SHIFT;
        debug_assert!(value <= Self::BITS);
        Self::from_usize(value)
    }
}

impl<C: cfg::Config> Pack<C> for () {
    const BITS: usize = 0;
    const LEN: usize = 0;
    const SHIFT: usize = 0;
    const MASK: usize = 0;

    type Prev = ();

    fn as_usize(&self) -> usize {
        unreachable!()
    }
    fn from_usize(_val: usize) -> Self {
        unreachable!()
    }

    fn pack(&self, _to: usize) -> usize {
        unreachable!()
    }

    fn from_packed(_from: usize) -> Self {
        unreachable!()
    }
}

#[inline(always)]
pub(crate) fn exponential_backoff(exp: &mut usize) {
    use crate::sync::{atomic, yield_now};

    /// Maximum exponent we can back off to.
    const MAX_EXPONENT: usize = 8;

    // Issue 2^exp pause instructions.
    for _ in 0..(1 << *exp) {
        atomic::spin_loop_hint();
    }

    if *exp >= MAX_EXPONENT {
        // If we have reached the max backoff, also yield to the scheduler
        // explicitly.
        yield_now();
    } else {
        // Otherwise, increment the exponent.
        *exp += 1;
    }
}

#[cfg(test)]
pub(crate) use self::tests::util as test_util;
#[cfg(test)]
mod tests;
