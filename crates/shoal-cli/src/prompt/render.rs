use std::time::Duration;

use shoal_config::schema::{PromptStyle, Rgb};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    Bash,
    Zsh,
}

/// One segment's plain text, before escaping, and its color.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub color: Rgb,
    pub text: String,
}

/// What follows the segments, ending with the prompt character.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tail {
    /// How long the last command ran; shown in `took_color` when over 2 s.
    pub took: Duration,
    pub jobs: u32,
    pub took_color: Rgb,
    pub jobs_color: Rgb,
    /// `accent`, or `red` after a failure.
    pub character_color: Rgb,
}

/// Commands shorter than this show no duration.
const SLOW: Duration = Duration::from_secs(2);
const SEPARATOR: char = '\u{e0b0}';
/// Rounded ends: a left half circle before the first block, a right one
/// after the last.
const START_CAP: char = '\u{e0b6}';
const END_CAP: char = '\u{e0b4}';
const CHARACTER: char = '❯';

/// One prompt line for `shell`. In `powerline` style each segment is a block of
/// its color with `text_color` text, joined by arrows between rounded ends; in
/// `lean` style the segment text is colored.
pub fn render(
    shell: Shell,
    style: PromptStyle,
    segments: &[Segment],
    text_color: Rgb,
    tail: &Tail,
) -> String {
    let mut line = Line::new(shell);
    match style {
        PromptStyle::Powerline => {
            if let Some(first) = segments.first() {
                line.sgr("49");
                line.sgr(&foreground(first.color));
                line.text(&START_CAP.to_string());
            }
            for (index, segment) in segments.iter().enumerate() {
                line.sgr(&background(segment.color));
                line.sgr(&foreground(text_color));
                line.text(&format!(" {} ", segment.text));
                line.sgr(&foreground(segment.color));
                let join = match segments.get(index + 1) {
                    Some(next) => {
                        line.sgr(&background(next.color));
                        SEPARATOR
                    }
                    None => {
                        line.sgr("49");
                        END_CAP
                    }
                };
                line.text(&join.to_string());
            }
        }
        PromptStyle::Lean => {
            for (index, segment) in segments.iter().enumerate() {
                if index > 0 {
                    line.text(" ");
                }
                line.sgr(&foreground(segment.color));
                line.text(&segment.text);
            }
        }
    }
    line.sgr("0");
    line.text(" ");
    if tail.took > SLOW {
        line.sgr(&foreground(tail.took_color));
        line.text(&format!("took {}", duration(tail.took)));
        line.sgr("0");
        line.text(" ");
    }
    if tail.jobs > 0 {
        line.sgr(&foreground(tail.jobs_color));
        line.text(&format!("✦{}", tail.jobs));
        line.sgr("0");
        line.text(" ");
    }
    line.sgr(&foreground(tail.character_color));
    line.text(&CHARACTER.to_string());
    line.sgr("0");
    line.text(" ");
    line.finish()
}

fn foreground(Rgb([r, g, b]): Rgb) -> String {
    format!("38;2;{r};{g};{b}")
}

fn background(Rgb([r, g, b]): Rgb) -> String {
    format!("48;2;{r};{g};{b}")
}

/// `12s`, `3m 4s` or `1h 5m`.
fn duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    match (seconds / 3600, seconds / 60 % 60, seconds % 60) {
        (0, 0, seconds) => format!("{seconds}s"),
        (0, minutes, seconds) => format!("{minutes}m {seconds}s"),
        (hours, minutes, _) => format!("{hours}h {minutes}m"),
    }
}

/// A prompt string being built: escape sequences go inside the shell's
/// non-printing markers, and text is escaped so the shell shows it literally.
struct Line {
    shell: Shell,
    out: String,
    in_escape: bool,
}

impl Line {
    fn new(shell: Shell) -> Self {
        Self {
            shell,
            out: String::new(),
            in_escape: false,
        }
    }

    fn sgr(&mut self, parameters: &str) {
        if !self.in_escape {
            self.out.push_str(match self.shell {
                Shell::Bash => "\\[",
                Shell::Zsh => "%{",
            });
            self.in_escape = true;
        }
        self.out.push_str("\x1b[");
        self.out.push_str(parameters);
        self.out.push('m');
    }

    fn text(&mut self, text: &str) {
        self.close_escape();
        for character in text.chars() {
            match character {
                // A control character in a directory name could drive the terminal.
                character if character.is_control() => self.out.push('\u{fffd}'),
                // Both shells expand these in a prompt: bash by default
                // (promptvars), zsh with prompt_subst.
                '\\' | '$' | '`' => {
                    self.out.push('\\');
                    self.out.push(character);
                }
                '%' if self.shell == Shell::Zsh => self.out.push_str("%%"),
                character => self.out.push(character),
            }
        }
    }

    fn close_escape(&mut self) {
        if self.in_escape {
            self.out.push_str(match self.shell {
                Shell::Bash => "\\]",
                Shell::Zsh => "%}",
            });
            self.in_escape = false;
        }
    }

    fn finish(mut self) -> String {
        self.close_escape();
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BG: Rgb = Rgb([1, 2, 3]);
    const BLUE: Rgb = Rgb([0, 0, 255]);
    const GREEN: Rgb = Rgb([0, 255, 0]);
    const ACCENT: Rgb = Rgb([9, 9, 9]);

    fn tail(took: Duration, jobs: u32) -> Tail {
        Tail {
            took,
            jobs,
            took_color: Rgb([7, 7, 7]),
            jobs_color: Rgb([8, 8, 8]),
            character_color: ACCENT,
        }
    }

    fn segments() -> Vec<Segment> {
        vec![
            Segment {
                color: BLUE,
                text: "~/a".into(),
            },
            Segment {
                color: GREEN,
                text: "b".into(),
            },
        ]
    }

    #[test]
    fn powerline_joins_blocks_with_colored_separators_between_rounded_ends() {
        let line = render(
            Shell::Bash,
            PromptStyle::Powerline,
            &segments(),
            BG,
            &tail(Duration::ZERO, 0),
        );
        assert_eq!(
            line,
            "\\[\x1b[49m\x1b[38;2;0;0;255m\\]\u{e0b6}\
             \\[\x1b[48;2;0;0;255m\x1b[38;2;1;2;3m\\] ~/a \
             \\[\x1b[38;2;0;0;255m\x1b[48;2;0;255;0m\\]\u{e0b0}\
             \\[\x1b[48;2;0;255;0m\x1b[38;2;1;2;3m\\] b \
             \\[\x1b[38;2;0;255;0m\x1b[49m\\]\u{e0b4}\
             \\[\x1b[0m\\] \\[\x1b[38;2;9;9;9m\\]❯\\[\x1b[0m\\] "
        );
    }

    #[test]
    fn lean_colors_the_text_and_zsh_uses_its_markers() {
        let line = render(
            Shell::Zsh,
            PromptStyle::Lean,
            &segments(),
            BG,
            &tail(Duration::ZERO, 0),
        );
        assert_eq!(
            line,
            "%{\x1b[38;2;0;0;255m%}~/a %{\x1b[38;2;0;255;0m%}b\
             %{\x1b[0m%} %{\x1b[38;2;9;9;9m%}❯%{\x1b[0m%} "
        );
    }

    #[test]
    fn shows_slow_commands_and_jobs_before_the_character() {
        let line = render(
            Shell::Zsh,
            PromptStyle::Lean,
            &[],
            BG,
            &tail(Duration::from_millis(12_400), 2),
        );
        assert_eq!(
            line,
            "%{\x1b[0m%} %{\x1b[38;2;7;7;7m%}took 12s%{\x1b[0m%} \
             %{\x1b[38;2;8;8;8m%}✦2%{\x1b[0m%} %{\x1b[38;2;9;9;9m%}❯%{\x1b[0m%} "
        );
        let quick = render(Shell::Zsh, PromptStyle::Lean, &[], BG, &tail(SLOW, 0));
        assert!(!quick.contains("took"), "{quick}");
    }

    #[test]
    fn durations_read_naturally() {
        for (seconds, shown) in [
            (3, "3s"),
            (59, "59s"),
            (60, "1m 0s"),
            (754, "12m 34s"),
            (3_900, "1h 5m"),
        ] {
            assert_eq!(duration(Duration::from_secs(seconds)), shown);
        }
    }

    #[test]
    fn escapes_text_for_each_shell() {
        let text = |shell| {
            let mut line = Line::new(shell);
            line.text("$(x) `y` \\ 100% \x1b[31m\n");
            line.finish()
        };
        assert_eq!(
            text(Shell::Bash),
            "\\$(x) \\`y\\` \\\\ 100% \u{fffd}[31m\u{fffd}"
        );
        assert_eq!(
            text(Shell::Zsh),
            "\\$(x) \\`y\\` \\\\ 100%% \u{fffd}[31m\u{fffd}"
        );
    }
}
