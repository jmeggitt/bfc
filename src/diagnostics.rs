//! Human-readable warnings and errors for the CLI.

use ansi_term::ANSIStrings;
use ansi_term::Colour::{Purple, Red};
use ansi_term::Style;
use std::fmt;

use crate::bfir::Position;

#[derive(Debug, PartialEq, Eq)]
pub struct Warning {
    pub message: String,
    pub position: Option<Position>,
}

/// The severity of the Info.
#[derive(Debug)]
pub enum Level {
    Warning,
    Error,
}

/// Info represents a message to the user, a warning or an error with
/// an optional reference to a position in the BF source.
#[derive(Debug)]
pub struct Info {
    pub level: Level,
    pub filename: Option<String>,
    pub message: String,
    pub position: Option<Position>,
    pub source: Option<String>,
    pub line_col: Option<(u64, u64)>,
}

impl Info {
    pub fn warn(msg: impl Into<String>) -> Self {
        Info {
            level: Level::Warning,
            filename: None,
            message: msg.into(),
            position: None,
            source: None,
            line_col: None
        }
    }

    pub fn error(msg: impl Into<String>) -> Self {
        Info {
            level: Level::Error,
            filename: None,
            message: msg.into(),
            position: None,
            source: None,
            line_col: None
        }
    }
}

impl fmt::Display for Info {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        let mut file_text = match &self.filename {
            Some(v) => v.clone(),
            None => String::from(""),
        };

        // Find line and column offsets, if we have an index.
        let offsets = match (&self.position, &self.line_col) {
            (&Some(range), &Some((line_idx, column_idx))) => {
                debug_assert!(range.start <= range.end);

                // let (line_idx, column_idx) = position(source, range.start);
                //
                file_text = file_text + &format!(":{}:{}", line_idx + 1, column_idx + 1);
                Some((column_idx, range.end - range.start))
            }
            _ => None,
        };

        let (color, level_text) = match self.level {
            Level::Warning => (Purple, " warning: "),
            Level::Error => (Red, " error: "),
        };

        let mut context_line = "".to_owned();
        let mut caret_line = "".to_owned();
        if let (Some((column_idx, width)), &Some(ref source)) = (offsets, &self.source) {
            // The faulty line of code.
            // let line = source.split('\n').nth(line_idx).unwrap();
            // context_line = "\n".to_owned() + line;
            context_line = source.clone();

            // Highlight the faulty characters on that line.
            caret_line += "\n";
            for _ in 0..column_idx {
                caret_line += " ";
            }
            caret_line += "^";
            if width > 0 {
                for _ in 0..width {
                    caret_line += "~";
                }
            }
        }

        let bold = Style::new().bold();
        let default = Style::default();
        let strings = [
            bold.paint(file_text),
            color.bold().paint(level_text),
            bold.paint(self.message.clone()),
            default.paint(context_line),
            color.bold().paint(caret_line),
        ];
        write!(f, "{}", ANSIStrings(&strings))
    }
}
