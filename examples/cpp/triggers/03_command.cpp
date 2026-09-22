// SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
// SPDX-License-Identifier: Apache-2.0

// Run a shell command for each notification.
//
// command() runs what you give it with the notification described in
// environment variables: AVISO_EVENT_TYPE, AVISO_SEQUENCE, one
// AVISO_IDENTIFIER_<FIELD> per identifier field, and AVISO_NOTIFICATION_JSON
// for the whole thing. That is how a notification reaches a script that knows
// nothing about aviso.
//
// One thing to know: the command's own stdout is captured and dropped, so a
// bare echo shows nothing. Write to a file instead, as this example does.
//
// The knobs shown here matter in real use. timeout_secs() bounds a command
// that hangs. retries() covers a flaky one. required(false) lets the watch
// carry on if the command fails; required(true) would end it.
//
// Expect: the file the command appended to, one line per notification.

#include "../common.hpp"

#include <atomic>
#include <fstream>
#include <iostream>
#include <string>

namespace {

constexpr const char* kOutPath = "announced.txt";
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

    aviso::Trigger command = aviso::Trigger::command(
        std::string(R"(echo "seq $AVISO_SEQUENCE for $AVISO_EVENT_TYPE on $AVISO_IDENTIFIER_DATE" >> )") +
        kOutPath);
    command.label("announce").timeout_secs(5).retries(1).required(false);

    aviso::WatchRequest request(example::kEventType);
    request.add_trigger(std::move(command));

    CountOnly handler;
    aviso::Watch watch = client.watch(request, handler);
    const int rc = example::finish(watch, handler);

    std::cout << "contents of " << kOutPath << ":\n";
    std::ifstream in(kOutPath);
    for (std::string line; std::getline(in, line);) {
      std::cout << "  " << line << '\n';
    }
    return rc;
  });
}
