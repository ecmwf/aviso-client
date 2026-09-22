// SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
// SPDX-License-Identifier: Apache-2.0

// Publish several notifications in one call.
//
// notify_many() sends them concurrently and returns one result per input, in
// input order. A failing item does not throw; it shows up in the results as
// an "error" entry, so one bad notification cannot hide the rest.
//
// Expect: a JSON array with one entry per notification.

#include "../common.hpp"

#include <iostream>
#include <string>

int main() {
  return example::run([] {
    aviso::Client client = example::connect();

    // One object per notification: event_type, identifier, optional payload.
    // The third one has a deliberately bad time so you can see what a
    // per-item failure looks like next to two successes.
    const std::string batch = R"([
      {"event_type": "test_event",
       "identifier": {"date": "20260101", "time": "0000"},
       "payload": {"step": 0}},
      {"event_type": "test_event",
       "identifier": {"date": "20260101", "time": "0600"},
       "payload": {"step": 6}},
      {"event_type": "test_event",
       "identifier": {"date": "20260101", "time": "not-a-time"}}
    ])";

    // 0 lets the client pick how many requests to keep in flight.
    const std::string results = client.notify_many(batch, 0);
    std::cout << results << '\n';
    return 0;
  });
}
