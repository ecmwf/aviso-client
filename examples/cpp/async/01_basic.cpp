// SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
// SPDX-License-Identifier: Apache-2.0

// The async verbs: the same calls, returning std::future.
//
// notify, notify_json, schema and schema_for each have an _async twin that
// returns at once with a std::future. Nothing else changes: the same
// arguments, the same result string, and an aviso::Error thrown from get()
// instead of from the call. Use them when you have other work to do while a
// request is in flight, or when you want several requests to overlap.
// notify_many has no twin; it is already concurrent inside.
//
// Expect: both results printed, then how long the pair took together.

#include "../common.hpp"

#include <chrono>
#include <future>
#include <iostream>
#include <map>
#include <string>

int main() {
  return example::run([] {
    aviso::Client client = example::connect();
    const std::map<std::string, std::string> identifier = {{"date", "20260101"},
                                                          {"time", "0000"}};

    const auto started = std::chrono::steady_clock::now();

    // Both requests are on the wire before either get() is called.
    std::future<std::string> published = client.notify_async(example::kEventType, identifier);
    std::future<std::string> catalog = client.schema_async();

    // get() blocks for that one result and rethrows its error, if any.
    std::cout << "notify: " << published.get() << '\n';
    std::cout << "schema: " << catalog.get().substr(0, 60) << "...\n";

    const auto elapsed = std::chrono::duration_cast<std::chrono::milliseconds>(
        std::chrono::steady_clock::now() - started);
    std::cout << "both done in " << elapsed.count() << " ms\n";
    return 0;
  });
}
