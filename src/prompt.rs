/// Asks the user to confirm a destructive operation.
pub trait Confirm {
    fn confirm(&mut self, message: &str) -> anyhow::Result<bool>;
}
