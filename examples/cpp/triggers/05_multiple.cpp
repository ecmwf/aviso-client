// SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
// SPDX-License-Identifier: Apache-2.0

// Several triggers on one watch, and what "required" means.
//
// Triggers run in the order you add them, for every notification, before
// your handler sees it. A required trigger that fails ends the watch with a
// trigger error. An optional one that fails is skipped and the watch carries
// on. Mixing them lets you say which side effects are essential and which
// are best effort.
//
// Be clear about what "skipped" means from C++: nothing tells you. There is
// no callback and no log you can read; the notification simply reaches your
// handler as if the trigger had worked. If you need to know when a trigger
// fails, make it required, or have the trigger leave its own trace.
//
// Here: log to a file (required, it is the record), echo to stdout
// (optional), and a command that always fails (optional), to show that the
// failure is not fatal.
//
// Expect: echo lines for three notifications, the log file printed, and a
// clean exit. Change flaky to required(true) to see the watch end on its
// first failure instead, with exit code 1.

#include "../common.hpp"

#include <atomic>
#include <fstream>
#include <iostream>
#include <string>

namespace {

constexpr const char* kLogPath = "multi.log";
constexpr int kStopAfter = 3;

class CountOnly : public example::Handler {
 public:
  bool on_notification(const aviso::Notification&) override {
    return seen_.fetch_add(1) + 1 < kStopAfter;
  }

 private:
  std::atomic<int> seen_{0};
};

}  // namespace

int main() {
  return example::run([] {
    aviso::Client client = example::connect();

    aviso::Trigger record = aviso::Trigger::log(kLogPath);
    record.label("record").required(true);

    aviso::Trigger show = aviso::Trigger::echo();
    show.label("show").required(false);

    // 'false' exits 1 every time. Optional, so it cannot end the watch.
    aviso::Trigger flaky = aviso::Trigger::command("false");
    flaky.label("always-fails").required(false).retries(0);

    aviso::WatchRequest request(example::kEventType);
    request.add_trigger(std::move(record))
        .add_trigger(std::move(show))
        .add_trigger(std::move(flaky));

    CountOnly handler;
    aviso::Watch watch = client.watch(request, handler);
    // With flaky optional, this returns 0: the watch ended because the
    // handler said so. With flaky required, it reports the trigger error.
    const int rc = example::finish(watch, handler);

    std::cout << "contents of " << kLogPath << ":\n";
    std::ifstream in(kLogPath);
    for (std::string line; std::getline(in, line);) {
      std::cout << "  " << line << '\n';
    }
    return rc;
  });
}
