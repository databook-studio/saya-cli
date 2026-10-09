fn finite_seconds(limit: BudgetLimit) -> Option<std::time::Duration> {
    match limit {
        BudgetLimit::Finite(seconds) => Some(std::time::Duration::from_secs(seconds.get())),
        BudgetLimit::Unlimited => None,
    }
}
