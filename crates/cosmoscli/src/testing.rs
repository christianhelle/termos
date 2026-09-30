//! In-memory fakes for testing command handlers.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::{Duration, Instant};

pub use cosmos_core::testing::*;

use crate::interactive::Clock;
use crate::prompt::{Confirm, LineReader, Picker};

/// Answers every confirmation the same way and records what was asked.
#[derive(Clone, Default)]
pub struct ScriptedConfirm {
    pub answer: bool,
    pub asked: Rc<RefCell<Vec<String>>>,
}

impl ScriptedConfirm {
    pub fn answering(answer: bool) -> Self {
        ScriptedConfirm {
            answer,
            ..Default::default()
        }
    }
}

impl Confirm for ScriptedConfirm {
    fn confirm(&mut self, message: &str) -> anyhow::Result<bool> {
        self.asked.borrow_mut().push(message.to_string());
        Ok(self.answer)
    }
}

/// Hands out scripted lines, then reports the end of input, recording each prompt shown.
#[derive(Clone, Default)]
pub struct ScriptedLines {
    pub lines: Rc<RefCell<VecDeque<String>>>,
    pub prompts: Rc<RefCell<Vec<String>>>,
}

impl ScriptedLines {
    pub fn new(lines: &[&str]) -> Self {
        ScriptedLines {
            lines: Rc::new(RefCell::new(lines.iter().map(|l| l.to_string()).collect())),
            ..Default::default()
        }
    }
}

impl LineReader for ScriptedLines {
    fn read_line(&mut self, prompt: &str) -> anyhow::Result<Option<String>> {
        self.prompts.borrow_mut().push(prompt.to_string());
        Ok(self.lines.borrow_mut().pop_front())
    }
}

/// Gives scripted answers to pickers and records the items each one showed.
#[derive(Clone, Default)]
pub struct ScriptedPicker {
    pub answers: Rc<RefCell<VecDeque<Option<usize>>>>,
    pub shown: Rc<RefCell<Vec<Vec<String>>>>,
}

impl ScriptedPicker {
    pub fn answering(answers: &[Option<usize>]) -> Self {
        ScriptedPicker {
            answers: Rc::new(RefCell::new(answers.iter().copied().collect())),
            ..Default::default()
        }
    }
}

impl Picker for ScriptedPicker {
    fn pick(&mut self, _prompt: &str, items: &[String]) -> anyhow::Result<Option<usize>> {
        self.shown.borrow_mut().push(items.to_vec());
        Ok(self.answers.borrow_mut().pop_front().flatten())
    }
}

/// A clock that moves forward by a fixed step each time it is read.
pub struct FakeClock {
    now: Cell<Instant>,
    step: Duration,
}

impl FakeClock {
    pub fn stepping(step: Duration) -> Self {
        FakeClock {
            now: Cell::new(Instant::now()),
            step,
        }
    }

    pub fn frozen() -> Self {
        Self::stepping(Duration::ZERO)
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        let now = self.now.get();
        self.now.set(now + self.step);
        now
    }
}
