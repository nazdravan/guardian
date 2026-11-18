//! A yes/no question in a modal; "No" is the safe default.

/// Shows `question`; returns `Some(true)` on Yes, `Some(false)` on No or
/// Escape, `None` while open.
pub fn ask(ui: &mut egui::Ui, id: &str, question: &str) -> Option<bool> {
    let mut answer = None;
    let modal = egui::Modal::new(ui.scope_id().with(id)).show(ui.ctx(), |ui| {
        ui.set_max_width(420.0);
        ui.label(question);
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let no = ui.button("No");
            if ui.button("Yes").clicked() {
                answer = Some(true);
            }
            if no.clicked() {
                answer = Some(false);
            }
        });
    });
    if modal.should_close() && answer.is_none() {
        answer = Some(false);
    }
    answer
}
