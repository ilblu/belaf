use kdam::{term, tqdm, BarExt, Column, RichProgress};
use owo_colors::{OwoColorize, Rgb};
use spinoff::{spinners, Color as SpinoffColor, Spinner};

pub const ICON_SUCCESS: &str = "✓";
pub const ICON_ERROR: &str = "✗";
pub const ICON_WARNING: &str = "⚠";
pub const ICON_INFO: &str = "ℹ";
pub const ICON_ARROW: &str = "▶";
pub const ICON_POINTER: &str = "→";
pub const SEPARATOR: &str = "─";
pub const LOGO: &str = "🐱";

pub fn primary() -> Rgb {
    Rgb(114, 227, 173)
}

pub fn error() -> Rgb {
    Rgb(202, 50, 20)
}

pub fn warning() -> Rgb {
    Rgb(245, 158, 11)
}

pub fn info() -> Rgb {
    Rgb(59, 130, 246)
}

pub fn success() -> Rgb {
    Rgb(114, 227, 173)
}

pub fn primary_spinoff() -> SpinoffColor {
    SpinoffColor::TrueColor {
        r: 114,
        g: 227,
        b: 173,
    }
}

pub fn dimmed(text: &str) -> String {
    format!("{}", text.dimmed())
}

pub fn separator(width: usize) -> String {
    format!("{}", SEPARATOR.repeat(width).dimmed())
}

pub fn header(text: &str) -> String {
    format!(
        "\n{} {}\n{}",
        LOGO,
        text.bold().color(primary()),
        separator(60)
    )
}

pub fn success_icon() -> String {
    format!("{}", ICON_SUCCESS.color(success()).bold())
}

pub fn error_icon() -> String {
    format!("{}", ICON_ERROR.color(error()).bold())
}

pub fn warning_icon() -> String {
    format!("{}", ICON_WARNING.color(warning()).bold())
}

pub fn info_icon() -> String {
    format!("{}", ICON_INFO.color(info()).bold())
}

pub fn arrow_icon() -> String {
    format!("{}", ICON_ARROW.color(primary()))
}

pub fn pointer_icon() -> String {
    format!("{}", ICON_POINTER.color(primary()))
}

pub fn success_message(msg: &str) -> String {
    format!("{} {}", success_icon(), msg)
}

pub fn error_message(msg: &str) -> String {
    format!("{} {}", error_icon(), msg)
}

pub fn warning_message(msg: &str) -> String {
    format!("{} {}", warning_icon(), msg)
}

pub fn info_message(msg: &str) -> String {
    format!("{} {}", info_icon(), msg)
}

pub fn step_message(msg: &str) -> String {
    format!("{} {}", pointer_icon(), msg.dimmed())
}

pub fn highlight(text: &str) -> String {
    format!("{}", text.color(primary()).bold())
}

pub fn url(text: &str) -> String {
    format!("{}", text.color(info()).underline())
}

pub fn code(text: &str) -> String {
    format!("{}", text.color(warning()))
}

pub fn create_spinner(msg: impl Into<String>) -> Spinner {
    Spinner::new(spinners::Arc, msg.into(), Some(primary_spinoff()))
}

pub struct ReleaseProgressBar {
    progress: RichProgress,
    is_tty: bool,
}

impl ReleaseProgressBar {
    pub fn new(total: usize, message: &str) -> Self {
        use std::io::{stderr, IsTerminal};

        let is_tty = stderr().is_terminal();
        term::init(is_tty);

        let bar = tqdm!(total = total, animation = "arrow", ncols = 40);

        let progress = RichProgress::new(
            bar,
            vec![
                Column::Text(format!("  {} ", "└─".dimmed())),
                Column::Animation,
                Column::Percentage(1),
                Column::Text(format!(" {} ", message.dimmed())),
                Column::Text("(".dimmed().to_string()),
                Column::CountTotal,
                Column::Text(")".dimmed().to_string()),
            ],
        );

        Self { progress, is_tty }
    }

    pub fn update(&mut self, position: usize) {
        if self.is_tty {
            let _ = self.progress.update_to(position);
        }
    }

    pub fn finish(self) {
        if self.is_tty {
            drop(self.progress);
        }
    }
}

pub struct PhaseSpinner {
    spinner: Option<Spinner>,
}

impl PhaseSpinner {
    pub fn new(message: impl Into<String>) -> Self {
        use std::io::{stderr, IsTerminal};

        let spinner = if stderr().is_terminal() {
            Some(Spinner::new(
                spinners::Arc,
                format!("  └─ {}", message.into()),
                Some(primary_spinoff()),
            ))
        } else {
            None
        };

        Self { spinner }
    }

    pub fn update(&mut self, message: impl Into<String>) {
        if let Some(ref mut spinner) = self.spinner {
            spinner.update_text(format!("  └─ {}", message.into()));
        }
    }

    pub fn success(self, message: impl Into<String>) {
        if let Some(mut spinner) = self.spinner {
            spinner.success(&message.into());
        }
    }

    pub fn finish(self) {
        if let Some(mut spinner) = self.spinner {
            spinner.clear();
        }
    }
}

pub fn print_phase(current: usize, total: usize, message: &str) {
    println!(
        "\n{} {}",
        format!("[{}/{}]", current, total).color(primary()).bold(),
        message.bold()
    );
}
