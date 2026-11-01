//! Bounding the work and memory of object parsing.
//!
//! By default, reading an object costs time proportional to its bytes and
//! allocates in proportion to them, without a way for the caller to stop it.
//! A [`ParseBudget`] passed to [`FromBytes::from_bytes_with_budget`] is
//! charged for every byte the parser scans and for every buffer it allocates
//! from the input, including the reads that lazy dictionaries and arrays
//! perform later on. The budget can refuse any charge, for example because a
//! limit is exhausted, a deadline passed or the operation was cancelled.
//!
//! A refused charge makes the read fail as if the data had ended. Because
//! the parser recovers from damaged data, a read could still produce an
//! object after a refusal, with the data behind the refusal missing. A
//! budget must therefore keep refusing once it has refused, and report so
//! from [`ParseBudget::exhausted`]: `from_bytes_with_budget` then returns
//! [`Exhausted`] instead of the object. A lazy read of a dictionary entry or
//! an array item returns `None` after a refusal; the caller checks
//! [`ParseBudget::exhausted`] to tell a refusal from damaged data.
//!
//! [`FromBytes::from_bytes_with_budget`]: crate::object::FromBytes::from_bytes_with_budget

/// The number of bytes a reader scans between two charges of work.
pub(crate) const WORK_WINDOW: usize = 4096;

/// The refusal of a charge.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Exhausted;

/// A caller's limit on the work and memory of reading objects.
///
/// All methods take `&self`, since lazy objects share the budget; implement
/// them with interior mutability.
pub trait ParseBudget: Sync {
    /// Charges `bytes` of scanned input. This is also the place to poll a
    /// deadline or a cancellation.
    fn work(&self, bytes: usize) -> Result<(), Exhausted>;

    /// Charges `bytes` of memory before the parser allocates it.
    fn allocate(&self, bytes: usize) -> Result<(), Exhausted>;

    /// Returns `bytes` that an earlier [`allocate`](Self::allocate) charged,
    /// once the parser frees them.
    fn release(&self, bytes: usize);

    /// Returns whether the budget refused a charge. Once it returns `true`,
    /// it must keep returning `true` and refuse every later charge.
    fn exhausted(&self) -> bool;
}

/// A reference to a budget, which readers hold by a thin pointer so that a
/// reader without a budget stays small.
#[derive(Copy, Clone)]
pub struct BudgetRef<'a>(&'a dyn ParseBudget);

impl<'a> BudgetRef<'a> {
    /// Refers to `budget`.
    pub fn new(budget: &'a dyn ParseBudget) -> Self {
        Self(budget)
    }
}

impl<'a> core::ops::Deref for BudgetRef<'a> {
    type Target = dyn ParseBudget + 'a;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl core::fmt::Debug for BudgetRef<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("BudgetRef")
    }
}

/// Charges `bytes` to `budget`, if any, before an allocation.
#[inline]
pub(crate) fn allocate(budget: Option<&BudgetRef<'_>>, bytes: usize) -> Option<()> {
    match budget {
        Some(budget) => budget.allocate(bytes).ok(),
        None => Some(()),
    }
}
