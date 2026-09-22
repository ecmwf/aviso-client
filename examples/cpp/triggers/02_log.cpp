// SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
// SPDX-License-Identifier: Apache-2.0

// Append every notification to a file.
//
// log() writes one JSON line per notification to the path you give it, and
// keeps appending across runs. It is the trigger to reach for when you want
// a durable record without writing any file handling yourself.
//
// Expect: three notifications land in notifications.log, then the file is
// printed. Run it twice and the file has six lines.

#include "../common.hpp"

#include <atomic>
#include <fstream>
#include <iostream>
#include <string>

namespace {

constexpr const char* kLogPath = "notifications.log";
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

    aviso::WatchRequest request(example::kEventType);
    request.add_trigger(aviso::Trigger::log(kLogPath));

    CountOnly handler;
    aviso::Watch watch = client.watch(request, handler);
    const int rc = example::finish(watch, handler);

    std::cout << "contents of " << kLogPath << ":\n";
    std::ifstream in(kLogPath);
    for (std::string line; std::getline(in, line);) {
      std::cout << "  " << line << '\n';
    }
    return rc;
  });
}
