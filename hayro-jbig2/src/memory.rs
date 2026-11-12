use crate::error::{DecodeError, Result, bail};

pub(crate) struct MemoryBudget {
    limit: usize,
    retained: usize,
    segment: usize,
}

impl MemoryBudget {
    pub(crate) fn new(limit: Option<usize>) -> Self {
        Self {
            limit: limit.unwrap_or(usize::MAX),
            retained: 0,
            segment: 0,
        }
    }

    pub(crate) fn charge(&mut self, bytes: usize) -> Result<()> {
        let used = self
            .retained
            .checked_add(self.segment)
            .and_then(|used| used.checked_add(bytes));

        match used {
            Some(used) if used <= self.limit => {
                self.segment += bytes;
                Ok(())
            }
            _ => bail!(DecodeError::MemoryLimitExceeded),
        }
    }

    pub(crate) fn release(&mut self, bytes: usize) {
        self.segment = self.segment.saturating_sub(bytes);
    }

    pub(crate) fn retain(&mut self, bytes: usize) {
        let bytes = bytes.min(self.segment);
        self.segment -= bytes;
        self.retained += bytes;
    }

    pub(crate) fn end_segment(&mut self) {
        self.segment = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_memory_is_released() {
        let mut budget = MemoryBudget::new(Some(100));
        budget.charge(60).unwrap();
        assert!(budget.charge(60).is_err());
        budget.end_segment();
        budget.charge(60).unwrap();
    }

    #[test]
    fn retained_memory_is_kept() {
        let mut budget = MemoryBudget::new(Some(100));
        budget.charge(60).unwrap();
        budget.retain(60);
        budget.end_segment();
        assert!(budget.charge(60).is_err());
        budget.charge(40).unwrap();
    }

    #[test]
    fn released_memory_can_be_charged_again() {
        let mut budget = MemoryBudget::new(Some(100));
        budget.charge(60).unwrap();
        budget.release(60);
        budget.charge(60).unwrap();
    }
}
