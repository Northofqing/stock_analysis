use std::sync::{Mutex, MutexGuard};

use crate::error::TdxError;

pub(crate) fn lock<'a, T>(
    mutex: &'a Mutex<T>,
    context: &str,
) -> Result<MutexGuard<'a, T>, TdxError> {
    mutex
        .lock()
        .map_err(|_| TdxError::InvalidData(format!("TDX {context} mutex is poisoned")))
}

pub(crate) fn lock_recover<'a, T>(mutex: &'a Mutex<T>, context: &str) -> MutexGuard<'a, T> {
    mutex.lock().unwrap_or_else(|poisoned| {
        crate::logw!("sync", "recovering poisoned TDX {} mutex", context);
        poisoned.into_inner()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    fn poisoned() -> Mutex<u8> {
        let mutex = Mutex::new(7);
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let _guard = mutex.lock().unwrap();
            panic!("poison test mutex");
        }));
        mutex
    }

    #[test]
    fn fallible_lock_reports_poison_with_context() {
        let error = lock(&poisoned(), "test state").unwrap_err();
        assert!(matches!(error, TdxError::InvalidData(_)));
        assert!(error.to_string().contains("test state"));
        assert!(error.to_string().contains("poisoned"));
    }

    #[test]
    fn compatible_lock_recovers_without_a_second_panic() {
        let mutex = poisoned();
        assert_eq!(*lock_recover(&mutex, "test state"), 7);
    }
}
