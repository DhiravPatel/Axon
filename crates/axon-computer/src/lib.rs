//! `axon-computer` — computer-use primitives.
//!
//! The screenshot → reason → action loop, typed and capability-gated.
//! Production agents that drive a browser or desktop want three things:
//!
//!   1. A **capability gate** (`Computer`) so the type checker knows
//!      which functions can move the mouse / press keys / read pixels.
//!   2. A **deterministic backend** for tests and replay — taking a
//!      real screenshot is non-deterministic; the [`MockDriver`] here
//!      records every action and serves pixel buffers from a script,
//!      so a test's exact action sequence is byte-stable.
//!   3. **Tainted-output flow** for screenshots — what the model sees
//!      is *untrusted data*. The host wraps the bytes in
//!      `Tainted<Image>` (Axon's existing taint type) so a screenshot
//!      can't be passed into a `system:` prompt without an explicit
//!      sanitize step.
//!
//! This crate ships the bookkeeping types + the mock driver. Real
//! Playwright / CDP / desktop-AT drivers plug into [`ComputerDriver`]
//! as separate, capability-audited crates.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    Screenshot,
    MouseMove { x: i32, y: i32 },
    Click { x: i32, y: i32, button: MouseButton },
    DoubleClick { x: i32, y: i32 },
    Drag { from: (i32, i32), to: (i32, i32), button: MouseButton },
    Scroll { dx: i32, dy: i32 },
    Type { text: String },
    Key { name: String },
    Wait { ms: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Screenshot {
    pub width: u32,
    pub height: u32,
    pub format: String, // "png" | "jpeg"
    /// Raw bytes. For the mock driver these are a deterministic PNG
    /// signature + a tiny payload, not real pixels.
    pub bytes: Vec<u8>,
    /// `true` so the host can wrap into `Tainted<Image>` at the
    /// boundary — pixel contents are untrusted model-visible data.
    pub tainted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComputerError {
    OutOfBounds { x: i32, y: i32, w: u32, h: u32 },
    UnsupportedKey(String),
    DriverDisconnected(String),
    Validation(String),
}

impl std::fmt::Display for ComputerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ComputerError::OutOfBounds { x, y, w, h } => {
                write!(f, "click ({x}, {y}) is outside the {w}x{h} screen")
            }
            ComputerError::UnsupportedKey(k) => write!(f, "unsupported key `{k}`"),
            ComputerError::DriverDisconnected(s) => write!(f, "driver disconnected: {s}"),
            ComputerError::Validation(s) => write!(f, "validation: {s}"),
        }
    }
}

impl std::error::Error for ComputerError {}

/// The pluggable backend. Implementations: [`MockDriver`] (in-tree,
/// deterministic), and downstream crates for Playwright / CDP / macOS
/// AT-SPI, etc. The trait is intentionally minimal so swapping drivers
/// is one line of host wiring.
pub trait ComputerDriver: Send {
    fn screenshot(&mut self) -> Result<Screenshot, ComputerError>;
    fn click(&mut self, x: i32, y: i32, button: MouseButton) -> Result<(), ComputerError>;
    fn double_click(&mut self, x: i32, y: i32) -> Result<(), ComputerError>;
    fn mouse_move(&mut self, x: i32, y: i32) -> Result<(), ComputerError>;
    fn drag(
        &mut self,
        from: (i32, i32),
        to: (i32, i32),
        button: MouseButton,
    ) -> Result<(), ComputerError>;
    fn scroll(&mut self, dx: i32, dy: i32) -> Result<(), ComputerError>;
    fn type_text(&mut self, text: &str) -> Result<(), ComputerError>;
    fn key(&mut self, name: &str) -> Result<(), ComputerError>;
    fn wait(&mut self, ms: u64) -> Result<(), ComputerError>;
    /// Audit hook — every successful action appends to this log.
    fn action_log(&self) -> &[Action];
}

/// In-tree deterministic driver. The screen is a fixed-size rectangle
/// initialized to a single solid color; screenshots return a small
/// PNG-header payload plus the color byte; every action is recorded
/// in the action log so tests can assert exact behavior.
pub struct MockDriver {
    pub width: u32,
    pub height: u32,
    pub fill_byte: u8,
    log: Vec<Action>,
    /// Pre-recorded screenshots returned in order, then the fill
    /// fallback. Empty for the default solid-color driver.
    scripted_frames: Vec<Screenshot>,
}

impl MockDriver {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            fill_byte: 0x80,
            log: Vec::new(),
            scripted_frames: Vec::new(),
        }
    }

    /// Scripted backend variant: the next N screenshots return the
    /// supplied frames in order, then fall back to the solid fill.
    pub fn with_frames(frames: Vec<Screenshot>) -> Self {
        let mut m = Self::new(
            frames.first().map(|f| f.width).unwrap_or(800),
            frames.first().map(|f| f.height).unwrap_or(600),
        );
        m.scripted_frames = frames;
        m
    }

    fn in_bounds(&self, x: i32, y: i32) -> Result<(), ComputerError> {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            Err(ComputerError::OutOfBounds {
                x,
                y,
                w: self.width,
                h: self.height,
            })
        } else {
            Ok(())
        }
    }

    fn validate_key(name: &str) -> Result<(), ComputerError> {
        const ALLOWED: &[&str] = &[
            "enter", "return", "tab", "esc", "escape", "space", "backspace",
            "delete", "left", "right", "up", "down", "home", "end", "pageup",
            "pagedown", "shift", "ctrl", "alt", "meta", "cmd",
        ];
        let lower = name.to_ascii_lowercase();
        // Single character keys are always OK.
        if lower.chars().count() == 1 {
            return Ok(());
        }
        if ALLOWED.contains(&lower.as_str()) {
            return Ok(());
        }
        // Function keys F1..F24.
        if let Some(rest) = lower.strip_prefix('f') {
            if let Ok(n) = rest.parse::<u32>() {
                if (1..=24).contains(&n) {
                    return Ok(());
                }
            }
        }
        Err(ComputerError::UnsupportedKey(name.to_string()))
    }

}

impl Default for MockDriver {
    fn default() -> Self {
        Self::new(800, 600)
    }
}

impl ComputerDriver for MockDriver {
    fn screenshot(&mut self) -> Result<Screenshot, ComputerError> {
        self.log.push(Action::Screenshot);
        if !self.scripted_frames.is_empty() {
            return Ok(self.scripted_frames.remove(0));
        }
        // 8-byte PNG signature + one byte for the fill color so a test
        // can verify the payload changed without decoding pixels.
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        bytes.push(self.fill_byte);
        Ok(Screenshot {
            width: self.width,
            height: self.height,
            format: "png".to_string(),
            bytes,
            tainted: true,
        })
    }

    fn click(&mut self, x: i32, y: i32, button: MouseButton) -> Result<(), ComputerError> {
        self.in_bounds(x, y)?;
        self.log.push(Action::Click { x, y, button });
        Ok(())
    }

    fn double_click(&mut self, x: i32, y: i32) -> Result<(), ComputerError> {
        self.in_bounds(x, y)?;
        self.log.push(Action::DoubleClick { x, y });
        Ok(())
    }

    fn mouse_move(&mut self, x: i32, y: i32) -> Result<(), ComputerError> {
        self.in_bounds(x, y)?;
        self.log.push(Action::MouseMove { x, y });
        Ok(())
    }

    fn drag(
        &mut self,
        from: (i32, i32),
        to: (i32, i32),
        button: MouseButton,
    ) -> Result<(), ComputerError> {
        self.in_bounds(from.0, from.1)?;
        self.in_bounds(to.0, to.1)?;
        self.log.push(Action::Drag { from, to, button });
        Ok(())
    }

    fn scroll(&mut self, dx: i32, dy: i32) -> Result<(), ComputerError> {
        self.log.push(Action::Scroll { dx, dy });
        Ok(())
    }

    fn type_text(&mut self, text: &str) -> Result<(), ComputerError> {
        if text.is_empty() {
            return Err(ComputerError::Validation("empty text".into()));
        }
        if text.len() > 4096 {
            return Err(ComputerError::Validation(
                "text > 4096 chars rejected; chunk it".into(),
            ));
        }
        self.log.push(Action::Type {
            text: text.to_string(),
        });
        Ok(())
    }

    fn key(&mut self, name: &str) -> Result<(), ComputerError> {
        Self::validate_key(name)?;
        self.log.push(Action::Key {
            name: name.to_string(),
        });
        Ok(())
    }

    fn wait(&mut self, ms: u64) -> Result<(), ComputerError> {
        if ms > 60_000 {
            return Err(ComputerError::Validation(
                "wait > 60s rejected; use a real schedule".into(),
            ));
        }
        self.log.push(Action::Wait { ms });
        Ok(())
    }

    fn action_log(&self) -> &[Action] {
        &self.log
    }
}

/// Thread-safe handle for the host. The `Arc<Mutex<…>>` indirection
/// matches how host bindings hold mutable driver state across calls.
pub type SharedDriver = Arc<std::sync::Mutex<Box<dyn ComputerDriver + Send>>>;

pub fn shared_mock(width: u32, height: u32) -> SharedDriver {
    Arc::new(std::sync::Mutex::new(Box::new(MockDriver::new(width, height))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_screenshot_returns_png_header_and_records_action() {
        let mut d = MockDriver::new(800, 600);
        let s = d.screenshot().unwrap();
        assert_eq!(&s.bytes[0..4], &[0x89, b'P', b'N', b'G']);
        assert_eq!(s.width, 800);
        assert!(s.tainted, "screenshots are tainted by default");
        assert_eq!(d.action_log(), &[Action::Screenshot]);
    }

    #[test]
    fn click_in_bounds_records_action() {
        let mut d = MockDriver::new(800, 600);
        d.click(100, 200, MouseButton::Left).unwrap();
        assert_eq!(
            d.action_log(),
            &[Action::Click {
                x: 100,
                y: 200,
                button: MouseButton::Left
            }]
        );
    }

    #[test]
    fn click_out_of_bounds_errors() {
        let mut d = MockDriver::new(800, 600);
        let err = d.click(900, 100, MouseButton::Left).unwrap_err();
        assert!(matches!(err, ComputerError::OutOfBounds { .. }));
        assert!(d.action_log().is_empty(), "rejected action not logged");
    }

    #[test]
    fn type_validates_length() {
        let mut d = MockDriver::new(800, 600);
        assert!(d.type_text("").is_err());
        assert!(d.type_text("hello").is_ok());
        let long: String = std::iter::repeat('x').take(5000).collect();
        assert!(d.type_text(&long).is_err());
    }

    #[test]
    fn key_validates_against_allowlist() {
        let mut d = MockDriver::new(800, 600);
        for k in ["enter", "Tab", "esc", "F1", "F24", "a"] {
            assert!(d.key(k).is_ok(), "{k}");
        }
        for k in ["unknown_key", "F25", "control_z"] {
            assert!(d.key(k).is_err(), "{k}");
        }
    }

    #[test]
    fn drag_validates_both_endpoints() {
        let mut d = MockDriver::new(800, 600);
        assert!(d.drag((10, 10), (200, 200), MouseButton::Left).is_ok());
        assert!(d
            .drag((10, 10), (1000, 200), MouseButton::Left)
            .is_err());
    }

    #[test]
    fn wait_caps_at_one_minute() {
        let mut d = MockDriver::new(800, 600);
        assert!(d.wait(5_000).is_ok());
        assert!(d.wait(120_000).is_err());
    }

    #[test]
    fn scripted_frames_pop_in_order() {
        let frame1 = Screenshot {
            width: 100,
            height: 100,
            format: "png".into(),
            bytes: vec![1, 2, 3],
            tainted: true,
        };
        let frame2 = Screenshot {
            width: 100,
            height: 100,
            format: "png".into(),
            bytes: vec![9, 9, 9],
            tainted: true,
        };
        let mut d = MockDriver::with_frames(vec![frame1.clone(), frame2.clone()]);
        assert_eq!(d.screenshot().unwrap().bytes, frame1.bytes);
        assert_eq!(d.screenshot().unwrap().bytes, frame2.bytes);
        // Third pulls from the fill backend.
        let s = d.screenshot().unwrap();
        assert_eq!(&s.bytes[0..4], &[0x89, b'P', b'N', b'G']);
    }

    #[test]
    fn action_log_preserves_order_across_kinds() {
        let mut d = MockDriver::new(800, 600);
        d.screenshot().unwrap();
        d.mouse_move(50, 50).unwrap();
        d.click(50, 50, MouseButton::Left).unwrap();
        d.type_text("hi").unwrap();
        d.key("enter").unwrap();
        let log = d.action_log();
        assert_eq!(log.len(), 5);
        assert!(matches!(log[0], Action::Screenshot));
        assert!(matches!(log[2], Action::Click { .. }));
        assert!(matches!(log[3], Action::Type { .. }));
    }

    #[test]
    fn round_trip_action_json() {
        let a = Action::Click {
            x: 100,
            y: 200,
            button: MouseButton::Right,
        };
        let j = serde_json::to_string(&a).unwrap();
        let back: Action = serde_json::from_str(&j).unwrap();
        assert_eq!(back, a);
    }
}
