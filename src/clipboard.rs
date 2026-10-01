//! The system clipboard.

/// The system clipboard, opened the first time something is copied.
///
/// It stays open, because on Linux the copied text is only there while it is.
#[derive(Default)]
pub struct Clipboard {
    system: Option<arboard::Clipboard>,
}

impl Clipboard {
    /// Puts text on the clipboard.
    pub fn copy(&mut self, text: String) -> anyhow::Result<()> {
        let system = match &mut self.system {
            Some(system) => system,
            None => self.system.insert(arboard::Clipboard::new()?),
        };
        system.set_text(text)?;
        Ok(())
    }
}
