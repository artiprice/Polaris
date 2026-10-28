//! The composer's model list, taken from the loaded configuration (`[nebius.effort]`), so the
//! GUI shows exactly the models Polaris is configured to use and carries no names of its own.

use polaris_config::EffortModels;
use polaris_ui_gui::ModelOption;

/// Short label: the id after its last `/` (the provider prefix adds no information in a menu).
fn label_of(id: &str) -> String {
    match id.rsplit_once('/') {
        Some((_, tail)) => tail.to_owned(),
        None => id.to_owned(),
    }
}

/// Distinct configured models in effort order (low → max) and the index of the model configured
/// for effort `high`, which the window selects at start.
#[must_use]
pub fn model_options(effort: &EffortModels) -> (Vec<ModelOption>, usize) {
    let mut options: Vec<ModelOption> = Vec::with_capacity(5);
    for id in [
        &effort.low,
        &effort.medium,
        &effort.high,
        &effort.xhigh,
        &effort.max,
    ] {
        if !options.iter().any(|option| option.id == id.as_str()) {
            options.push(ModelOption {
                id: id.as_str().to_owned(),
                label: label_of(id.as_str()),
            });
        }
    }
    let default = match options
        .iter()
        .position(|option| option.id == effort.high.as_str())
    {
        Some(index) => index,
        None => 0,
    };
    (options, default)
}
