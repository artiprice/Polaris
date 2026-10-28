use ratatui::style::{Color, Modifier, Style};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeMode {
    Dark,
    Terminal,
}

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub mode: ThemeMode,
    pub color: bool,
    pub rgb: bool,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            mode: ThemeMode::Dark,
            color: true,
            rgb: true,
        }
    }
}

impl Theme {
    fn shade(self, rgb: (u8, u8, u8), ansi: Color) -> Color {
        if !self.color {
            Color::Reset
        } else if self.rgb && self.mode == ThemeMode::Dark {
            Color::Rgb(rgb.0, rgb.1, rgb.2)
        } else {
            ansi
        }
    }
    pub fn base(self) -> Style {
        if self.mode == ThemeMode::Terminal || !self.color {
            Style::default()
        } else {
            Style::default()
                .fg(self.shade((232, 232, 232), Color::Reset))
                .bg(self.shade((0, 0, 0), Color::Black))
        }
    }
    pub fn muted(self) -> Style {
        if self.mode == ThemeMode::Terminal || !self.color {
            self.base()
        } else {
            self.base().fg(self.shade((133, 133, 133), Color::DarkGray))
        }
    }
    pub fn accent(self) -> Style {
        self.base()
            .fg(self.shade((118, 185, 0), Color::Green))
            .add_modifier(Modifier::BOLD)
    }
    pub fn rule(self) -> Style {
        self.base().fg(self.shade((52, 52, 52), Color::DarkGray))
    }
    /// A foreground-only overlay: preserve the surrounding surface and emphasis.
    pub fn selected(self) -> Style {
        Style::default().fg(self.shade((118, 185, 0), Color::Green))
    }
    /// Preserve source foregrounds and emphasis while making copied text visible.
    /// RGB dark surfaces blend 30% of the source foreground over the existing
    /// background. Whitespace uses the neutral text tint, including on gray prompts.
    /// Unknown terminal palettes use a stronger neutral indexed fallback.
    pub fn copy_selection(self, foreground: Color, background: Color, whitespace: bool) -> Style {
        if !self.color {
            return Style::default().add_modifier(Modifier::BOLD);
        }
        let background = if self.rgb && self.mode == ThemeMode::Dark {
            let (r, g, b) = match foreground {
                Color::Rgb(r, g, b) if !whitespace => (r, g, b),
                _ => (232, 232, 232),
            };
            let (br, bg, bb) = match background {
                Color::Rgb(r, g, b) => (r, g, b),
                _ => (0, 0, 0),
            };
            let blend = |fg: u8, bg: u8| ((u16::from(bg) * 7 + u16::from(fg) * 3 + 5) / 10) as u8;
            Color::Rgb(blend(r, br), blend(g, bg), blend(b, bb))
        } else {
            Color::Indexed(237)
        };
        Style::default().bg(background)
    }
    pub fn latest_button(self, hovered: bool) -> Style {
        if self.mode == ThemeMode::Dark && self.color {
            self.base().bg(if self.rgb {
                let gray = if hovered { 55 } else { 38 };
                Color::Rgb(gray, gray, gray)
            } else {
                Color::DarkGray
            })
        } else if hovered {
            self.base().add_modifier(Modifier::UNDERLINED)
        } else {
            self.base()
        }
    }
    pub fn sent_prompt(self) -> Style {
        if self.mode == ThemeMode::Dark && self.color {
            self.base().bg(if self.rgb {
                Color::Rgb(20, 20, 20)
            } else {
                Color::DarkGray
            })
        } else {
            self.base()
        }
    }
    /// A bounded interactive surface; lexical token styles stay foreground-only.
    pub fn control(self, hovered: bool, focused: bool) -> Style {
        if !self.color || self.mode == ThemeMode::Terminal {
            if focused || hovered {
                Style::default().add_modifier(Modifier::UNDERLINED)
            } else {
                Style::default()
            }
        } else if hovered || focused {
            Style::default().bg(if self.rgb {
                let v = if hovered { 24 } else { 25 };
                Color::Rgb(v, v, v)
            } else {
                Color::Indexed(237)
            })
        } else {
            Style::default()
        }
    }
    pub fn panel(self) -> Style {
        self.base()
    }
    pub fn mention(self) -> Style {
        Style::default().fg(self.shade((249, 226, 175), Color::Yellow))
    }
    pub fn tool_title(self, hovered: bool) -> Style {
        self.base().fg(self.shade(
            if hovered {
                (255, 253, 230)
            } else {
                (249, 226, 175)
            },
            if hovered {
                Color::LightYellow
            } else {
                Color::Yellow
            },
        ))
    }
    pub fn diff_added(self) -> Style {
        if !self.color {
            self.base()
        } else {
            self.base()
                .fg(self.shade((145, 205, 102), Color::LightGreen))
                .bg(if self.rgb {
                    Color::Rgb(7, 35, 12)
                } else {
                    Color::Indexed(22)
                })
        }
    }
    pub fn diff_removed(self) -> Style {
        if !self.color {
            self.base()
        } else {
            self.base()
                .fg(self.shade((245, 150, 150), Color::LightRed))
                .bg(if self.rgb {
                    Color::Rgb(50, 7, 9)
                } else {
                    Color::Indexed(52)
                })
        }
    }
    pub fn auto_mode(self) -> Style {
        self.base()
            .fg(self.shade((245, 194, 231), Color::LightMagenta))
    }
    pub fn plan_mode(self) -> Style {
        self.base().fg(self.shade((249, 226, 175), Color::Yellow))
    }
    /// Foreground only, so prompt backgrounds and selection surfaces are preserved.
    pub fn setting(self, hovered: bool) -> Style {
        Style::default().fg(self.shade(
            if hovered {
                (180, 210, 255)
            } else {
                (137, 180, 250)
            },
            if hovered {
                Color::LightCyan
            } else {
                Color::LightBlue
            },
        ))
    }
    pub fn goal_status(self) -> Style {
        self.base()
            .fg(self.shade((137, 180, 250), Color::LightBlue))
    }
    pub fn placeholder(self) -> Style {
        self.base().fg(self.shade((82, 82, 82), Color::DarkGray))
    }
    pub fn inactive(self) -> Style {
        self.base().fg(self.shade((94, 94, 94), Color::DarkGray))
    }
    pub fn error(self) -> Style {
        self.base()
            .fg(self.shade((172, 66, 66), Color::Red))
            .remove_modifier(Modifier::BOLD)
    }
    pub fn warning(self) -> Style {
        self.base()
            .fg(self.shade((229, 180, 84), Color::Yellow))
            .remove_modifier(Modifier::BOLD)
    }
    pub fn bold(self) -> Style {
        self.base().add_modifier(Modifier::BOLD)
    }
}
