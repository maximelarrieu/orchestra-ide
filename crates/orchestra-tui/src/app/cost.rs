//! The cost screen's keys.

use super::*;

impl App {
    pub(super) fn on_cost_char(&mut self, c: char) {
        match c {
            'm' => {
                self.cost.group = self.cost.next_group();
                self.cost.selected = 0;
                self.request_usage();
            }
            'p' => {
                self.cost.period = self.cost.period.next();
                self.cost.selected = 0;
                self.request_usage();
            }
            'u' => {
                self.cost.include_unmanaged = !self.cost.include_unmanaged;
                self.request_usage();
            }
            _ => {}
        }
    }
}
