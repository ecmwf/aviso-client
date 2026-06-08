// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use super::*;

#[test]
fn heartbeat_budget_30s_interval_returns_90s() {
    assert_eq!(
        super::heartbeat_starvation_budget(Duration::from_secs(30)),
        Duration::from_secs(90)
    );
}

#[test]
fn heartbeat_budget_10s_interval_returns_40s_via_plus_30() {
    assert_eq!(
        super::heartbeat_starvation_budget(Duration::from_secs(10)),
        Duration::from_secs(40)
    );
}

#[test]
fn heartbeat_budget_1s_interval_returns_31s_via_plus_30() {
    assert_eq!(
        super::heartbeat_starvation_budget(Duration::from_secs(1)),
        Duration::from_secs(31)
    );
}

#[test]
fn heartbeat_budget_zero_interval_returns_30s_floor() {
    assert_eq!(
        super::heartbeat_starvation_budget(Duration::ZERO),
        Duration::from_secs(30)
    );
}

#[test]
fn heartbeat_budget_absurd_interval_caps_at_136_years() {
    let cap = Duration::from_secs(u64::from(u32::MAX));
    assert_eq!(super::heartbeat_starvation_budget(Duration::MAX), cap);
    let near_max = Duration::from_secs(u64::MAX / 2);
    assert_eq!(super::heartbeat_starvation_budget(near_max), cap);
}
