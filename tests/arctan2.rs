use renderer::{runtime::WatchVm, watch::EngineNode};

fn eval_arctan2(y: f64, x: f64) -> f64 {
    let nodes: Vec<EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": y},
        {"value": x},
        {"func": "Arctan2", "args": [0, 1]}
    ]))
    .unwrap();
    WatchVm::new(&nodes).execute(2).unwrap()
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1e-12,
        "expected {expected}, got {actual}"
    );
}

#[test]
fn arctan2_uses_sonolus_y_then_x_argument_order_across_axes_and_quadrants() {
    let cases = [
        (1.0, 0.0, std::f64::consts::FRAC_PI_2),
        (0.0, 1.0, 0.0),
        (-1.0, 0.0, -std::f64::consts::FRAC_PI_2),
        (0.0, -1.0, std::f64::consts::PI),
        (1.0, 1.0, std::f64::consts::FRAC_PI_4),
        (1.0, -1.0, 3.0 * std::f64::consts::FRAC_PI_4),
        (-1.0, -1.0, -3.0 * std::f64::consts::FRAC_PI_4),
        (-1.0, 1.0, -std::f64::consts::FRAC_PI_4),
    ];

    for (y, x, expected) in cases {
        assert_close(eval_arctan2(y, x), expected);
    }
}

#[test]
fn arctan2_matches_the_oracle_fixture_in_execution_order() {
    let nodes: Vec<EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": 1}, {"value": 0}, {"func": "Arctan2", "args": [0, 1]},
        {"func": "DebugLog", "args": [2]},
        {"value": 0}, {"value": 1}, {"func": "Arctan2", "args": [4, 5]},
        {"func": "DebugLog", "args": [6]},
        {"value": -1}, {"value": 0}, {"func": "Arctan2", "args": [8, 9]},
        {"func": "DebugLog", "args": [10]},
        {"value": 0}, {"value": -1}, {"func": "Arctan2", "args": [12, 13]},
        {"func": "DebugLog", "args": [14]},
        {"value": 0}, {"value": 0}, {"func": "Arctan2", "args": [16, 17]},
        {"func": "DebugLog", "args": [18]},
        {"func": "Execute", "args": [3, 7, 11, 15, 19]}
    ]))
    .unwrap();

    let mut vm = WatchVm::new(&nodes);
    vm.execute(20).unwrap();
    let values: Vec<f64> = vm
        .debug_events
        .iter()
        .filter_map(|event| match event {
            renderer::runtime::DebugEvent::Log { value, .. } => Some(*value),
            renderer::runtime::DebugEvent::Pause { .. } => None,
        })
        .collect();
    let expected = [
        std::f64::consts::FRAC_PI_2,
        0.0,
        -std::f64::consts::FRAC_PI_2,
        std::f64::consts::PI,
        0.0,
    ];
    assert_eq!(values.len(), expected.len());
    for (actual, expected) in values.into_iter().zip(expected) {
        assert_close(actual, expected);
    }
}

#[test]
fn arctan2_rejects_a_non_binary_call() {
    let one_argument: Vec<EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": 1}, {"func": "Arctan2", "args": [0]}
    ]))
    .unwrap();
    assert!(WatchVm::new(&one_argument).execute(1).is_err());

    let three_arguments: Vec<EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": 1}, {"value": 2}, {"value": 3},
        {"func": "Arctan2", "args": [0, 1, 2]}
    ]))
    .unwrap();
    assert!(WatchVm::new(&three_arguments).execute(3).is_err());
}
