//! Appearance changes preserve mirrors, streams and window-local terminal state.
use shoal_config::{Action, Config, Palette};

use super::ShoalWindow;

impl ShoalWindow {
    pub fn apply_config(&mut self, config: &Config, palette: Palette) {
        if self.config.font != config.font {
            self.invalidate_font();
        }
        self.panes
            .apply_setup(crate::mirror::Setup::from_config(config, &palette));
        self.config = config.clone();
        self.palette = palette;
    }

    pub(super) fn zoom_font(&mut self, action: Action) {
        if self.zoom.change(action, self.config.font.size) {
            self.invalidate_font();
        }
    }

    fn invalidate_font(&mut self) {
        self.glyphs.clear();
        self.metrics = None;
        self.panes.invalidate_rows();
    }
}
