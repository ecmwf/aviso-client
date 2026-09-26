// SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
// SPDX-License-Identifier: Apache-2.0

// Listen with several watches through one handler.
//
// A WatchSet holds named watch requests, each with its own filter, start
// position and triggers. client.watch_many() starts all of them and calls one
// handler with the name of the watch each notification came from. Watches are
// read in turn, so a busy one cannot delay a quiet one. The callbacks run one
// at a time, so the handler needs no lock for its own state.
//
// When one watch fails, on_error() decides: return true to drop it and keep
// the others, or false (the default) to stop them all. This handler reports
// the failure and keeps going. on_end() still reports an error if every
// watch failed, so the program cannot finish quietly with nothing running.
//
// Run it in one terminal and 02_publish in another. Each publish matches both
// watches, since 02_publish sends the date the second one filters on.
//
// Expect: lines prefixed "all" and "january", then a clean exit once each
// watch has delivered three notifications.

#include "../common.hpp"

#include <iostream>
#include <map>
#include <string>

namespace {

constexpr int kStopAfter = 3;

class Printer : public example::MultiHandler {
 public:
  bool on_notification(const std::string& name,
                       const aviso::Notification& n) override {
    // Callbacks never overlap, so the map needs no lock.
    ++seen_[name];
    std::cout << name << "  #" << n.sequence() << "  " << n.identifier_json()
              << '\n';
    // Returning false stops every watch in the set.
    return seen_["all"] < kStopAfter || seen_["january"] < kStopAfter;
  }

  bool on_error(const std::string& name,
                const aviso::ErrorInfo& error) override {
    std::cerr << "watch " << name << " failed and was dropped\n";
    example::report(error);
    return true;
  }

 private:
  std::map<std::string, int> seen_;
};

}  // namespace

int main() {
  return example::run([] {
    aviso::Client client = example::connect();

    aviso::WatchRequest all(example::kEventType);
    aviso::WatchRequest january(example::kEventType);
    january.filter_json(R"({"date": "20260101"})");

    aviso::WatchSet watches;
    watches.add("all", all).add("january", january);

    Printer printer;
    std::cout << "listening with two watches on " << example::kEventType
              << ", stopping after " << kStopAfter << " from each\n";
    aviso::Watch watch = client.watch_many(watches, printer);

    // As for a single watch: finish() waits, and the Watch destructor stops
    // and waits for every watch in the set.
    return example::finish(watch, printer);
  });
}
