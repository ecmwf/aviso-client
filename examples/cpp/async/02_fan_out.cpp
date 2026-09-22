// SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
// SPDX-License-Identifier: Apache-2.0

// Fire many requests, then collect them.
//
// This is what the async verbs are for. Publishing ten notifications one
// after another costs ten round trips; publishing them as ten futures and
// then collecting costs about one. The pattern is: start everything, keep
// the futures, then get() them in order.
//
// For plain publishing, notify_many() does this for you and reports per-item
// failures without throwing (see basics/04). Reach for futures when the
// requests are not all publishes, or when you have work to do between
// starting them and collecting.
//
// Expect: ten publish results, then one schema, and a timing line.

#include "../common.hpp"

#include <chrono>
#include <cstdio>
#include <future>
#include <iostream>
#include <map>
#include <string>
#include <vector>

int main() {
  return example::run([] {
    aviso::Client client = example::connect();
    constexpr int kCount = 10;

    const auto started = std::chrono::steady_clock::now();

    std::vector<std::future<std::string>> publishes;
    publishes.reserve(kCount);
    for (int i = 0; i < kCount; ++i) {
      // Vary the time so each notification is distinct.
      char hhmm[5];
      std::snprintf(hhmm, sizeof hhmm, "%02d00", i);
      const std::map<std::string, std::string> identifier = {{"date", "20260102"},
                                                            {"time", hhmm}};
      publishes.push_back(client.notify_async(example::kEventType, identifier));
    }
    // A different kind of request, in flight alongside the publishes.
    std::future<std::string> catalog = client.schema_for_async(example::kEventType);

    // Collect. A failed publish throws here, from its own get(); the others
    // are unaffected. Wrap each get() in try/catch to keep going past one.
    int ok = 0;
    for (std::future<std::string>& f : publishes) {
      try {
        static_cast<void>(f.get());
        ++ok;
      } catch (const aviso::Error& error) {
        example::report(error);
      }
    }
    std::cout << ok << " of " << kCount << " published\n";
    std::cout << "schema for " << example::kEventType << " has "
              << catalog.get().size() << " bytes\n";

    const auto elapsed = std::chrono::duration_cast<std::chrono::milliseconds>(
        std::chrono::steady_clock::now() - started);
    std::cout << "all " << kCount + 1 << " requests done in " << elapsed.count() << " ms\n";
    return 0;
  });
}
