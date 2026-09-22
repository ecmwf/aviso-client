// SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
// SPDX-License-Identifier: Apache-2.0

// Listen to a stream and print what arrives.
//
// A listener is a handler object plus a watch request. The client calls
// on_notification() for each notification, on its own thread, and on_end()
// once when the watch stops. Returning false from on_notification() stops
// the watch; this example stops after three so it terminates on its own.
//
// on_end() matters. It is the only place a failed watch is reported, so a
// handler that ignores it turns every failure into a quiet exit. The shared
// example::Handler remembers the error and example::finish() turns it into
// the exit code, so the examples never have to repeat that.
//
// Run it in one terminal and 02_publish in another to see output.
//
// Expect: three lines, one per notification, then a clean exit.

#include "../common.hpp"

#include <atomic>
#include <iostream>

namespace {

constexpr int kStopAfter = 3;

class Printer : public example::Handler {
 public:
  bool on_notification(const aviso::Notification& n) override {
    const int seen = seen_.fetch_add(1) + 1;
    std::cout << "#" << n.sequence() << "  " << n.identifier_json() << "  "
              << n.payload_json() << '\n';
    return seen < kStopAfter;
  }

 private:
  std::atomic<int> seen_{0};
};

}  // namespace

int main() {
  return example::run([] {
    aviso::Client client = example::connect();

    aviso::WatchRequest request(example::kEventType);

    Printer printer;
    std::cout << "listening to " << example::kEventType << ", stopping after "
              << kStopAfter << " notifications\n";
    aviso::Watch watch = client.watch(request, printer);

    // finish() waits until the handler returns false or the watch fails,
    // then reports how it ended. See 05_filter to receive only some
    // notifications, and resilience/04 to stop from outside.
    return example::finish(watch, printer);
  });
}
