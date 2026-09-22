// SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
// SPDX-License-Identifier: Apache-2.0

// Publish one notification.
//
// A notification is an event type plus an identifier, the set of fields that
// say what this notification is about. The fields and their formats come
// from the schema (see 01_schema); here the stream wants a date and a time.
//
// Expect: the server's response. It carries a request id, which is what to
// quote if you ever need to ask the server's operators about a publish.

#include "../common.hpp"

#include <iostream>
#include <map>
#include <string>

int main() {
  return example::run([] {
    aviso::Client client = example::connect();

    // Identifier values are strings. Use notify_json() instead when a value
    // has structure, such as a polygon of [lat, lon] pairs.
    const std::map<std::string, std::string> identifier = {
        {"date", "20260101"},
        {"time", "0000"},
    };

    // The payload is optional free-form JSON that travels with the
    // notification. Listeners see it verbatim.
    const std::string payload = R"({"source": "cpp-example", "run": 1})";

    const std::string response =
        client.notify(example::kEventType, identifier, payload);
    std::cout << "published: " << response << '\n';
    return 0;
  });
}
