// SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
// SPDX-License-Identifier: Apache-2.0

// Stop a listener from outside, cleanly.
//
// The other listeners stop themselves by returning false. A real one runs
// until something outside says stop: Ctrl+C, a shutdown request, a timer.
// Watch::stop() is for that. It is safe to call from any thread, wait()
// returns soon after, and on_end() runs with no error, so a stopped watch
// looks exactly like one that ended on purpose.
//
// A signal handler may not call stop() itself; almost nothing is allowed
// inside one. The pattern that works is the one here: the handler sets a
// flag, and a small thread watches the flag and calls stop() when it turns.
// The same thread also enforces a deadline, so this example ends on its own
// after a few seconds if you do not press anything.
//
// Expect: whatever arrives while it runs, then "stopped by timer" or
// "stopped by Ctrl+C", then a clean exit with code 0.

#include "../common.hpp"

#include <atomic>
#include <chrono>
#include <csignal>
#include <iostream>
#include <optional>
#include <thread>

namespace {

constexpr std::chrono::seconds kDeadline(4);

// Written from the signal handler, read from the stopper thread. A signal
// handler may only touch a lock-free atomic, and this one is.
std::atomic<bool> g_interrupted{false};
static_assert(std::atomic<bool>::is_always_lock_free);

extern "C" void on_sigint(int) { g_interrupted.store(true); }

class Printer : public example::Handler {
 public:
  bool on_notification(const aviso::Notification& n) override {
    std::cout << "#" << n.sequence() << "  " << n.identifier_json() << '\n';
    return true;  // never stop from here; the outside decides
  }

  // Lets the stopper give up early if the watch ends on its own, for
  // example because the server refused it.
  void on_end(const std::optional<aviso::ErrorInfo>& error) override {
    example::Handler::on_end(error);
    ended.store(true);
  }

  std::atomic<bool> ended{false};
};

}  // namespace

int main() {
  return example::run([] {
    aviso::Client client = example::connect();

    std::signal(SIGINT, on_sigint);

    Printer printer;
    aviso::WatchRequest request(example::kEventType);
    aviso::Watch watch = client.watch(request, printer);
    std::cout << "listening; Ctrl+C to stop, or wait " << kDeadline.count() << " s\n";

    // The stopper: sleep in short steps until the flag turns or the
    // deadline passes, then tell the watch to end.
    std::thread stopper([&] {
      const auto deadline = std::chrono::steady_clock::now() + kDeadline;
      while (!g_interrupted.load() && !printer.ended.load() &&
             std::chrono::steady_clock::now() < deadline) {
        std::this_thread::sleep_for(std::chrono::milliseconds(100));
      }
      if (printer.ended.load()) {
        return;  // nothing left to stop
      }
      std::cout << (g_interrupted.load() ? "stopped by Ctrl+C\n" : "stopped by timer\n");
      watch.stop();
    });

    // finish() returns once stop() has taken effect. The handler's on_end()
    // has run by then, with no error.
    const int rc = example::finish(watch, printer);
    stopper.join();
    return rc;
  });
}
