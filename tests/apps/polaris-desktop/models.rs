//! Desktop Model Options (descriptor: specs/descriptors/ui_desktop_models.json).

use std::error::Error;

use polaris_config::{Layer, LayerInput, TrustGrant, load};
use polaris_desktop::models::model_options;

type TestResult = Result<(), Box<dyn Error>>;

fn ids(options: &[polaris_ui_gui::ModelOption]) -> Vec<&str> {
    options.iter().map(|option| option.id.as_str()).collect()
}

#[test]
fn options_follow_config_effort_order_without_duplicates() -> TestResult {
    let config = load(&[], &TrustGrant::none())?.config;
    let effort = &config.nebius.effort;
    let (options, _default) = model_options(effort);
    let mut expected: Vec<&str> = Vec::new();
    for id in [
        &effort.low,
        &effort.medium,
        &effort.high,
        &effort.xhigh,
        &effort.max,
    ] {
        if !expected.contains(&id.as_str()) {
            expected.push(id.as_str());
        }
    }
    assert_eq!(ids(&options), expected, "config order, each model once");
    assert!(
        options.len() < 5,
        "the defaults reuse one model for high and xhigh: {:?}",
        ids(&options)
    );
    Ok(())
}

#[test]
fn default_is_the_high_effort_model() -> TestResult {
    let config = load(&[], &TrustGrant::none())?.config;
    let (options, default) = model_options(&config.nebius.effort);
    let chosen = options.get(default).ok_or("default index out of range")?;
    assert_eq!(
        chosen.id,
        config.nebius.effort.high.as_str(),
        "high effort model"
    );
    Ok(())
}

#[test]
fn labels_are_the_last_path_segment() -> TestResult {
    let config = load(&[], &TrustGrant::none())?.config;
    let (options, _default) = model_options(&config.nebius.effort);
    for option in &options {
        let expected = match option.id.rsplit_once('/') {
            Some((_, tail)) => tail,
            None => option.id.as_str(),
        };
        assert_eq!(option.label, expected, "label of {}", option.id);
    }
    Ok(())
}

#[test]
fn user_override_changes_the_list() -> TestResult {
    let layer = LayerInput::parse(
        Layer::Global,
        "[nebius.effort]\nmax = \"acme/custom-max-model\"\n",
        None,
    )?;
    let config = load(&[layer], &TrustGrant::none())?.config;
    let (options, _default) = model_options(&config.nebius.effort);
    let listed = ids(&options);
    assert_eq!(
        listed.last(),
        Some(&"acme/custom-max-model"),
        "override is listed: {listed:?}"
    );
    let defaults = load(&[], &TrustGrant::none())?.config;
    assert!(
        !listed.contains(&defaults.nebius.effort.max.as_str()),
        "the replaced default is gone: {listed:?}"
    );
    Ok(())
}
