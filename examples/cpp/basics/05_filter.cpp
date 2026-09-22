// SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
// SPDX-License-Identifier: Apache-2.0

// Receive only the notifications you care about.
//
// A filter is a JSON object of identifier fields and the values you want.
// The server applies it, so notifications that do not match never reach
// your process. Fields you leave out match anything.
//
// Here the filter is a single date. 02_publish sends that date, so its
// notifications arrive. async/02_fan_out sends a different one, so its ten
// notifications go past without a trace.
//
// Run it in one terminal. In another, run async/02_fan_out first to see
// nothing happen, then 02_publish three times.
//
// Expect: three lines, all with the same date, then a clean exit.

#include "../common.hpp"

#include <atomic>
#include <iostream>

namespace {

constexpr int kStopAfter = 3;
constexpr const char* kWantedDate = "20260101";

class Printer : public example::Handler {
 public:
  bool on_notification(const aviso::Notification& n) override {
    std::cout << "#" << n.sequence() << "  " << n.identifier_json() << '\n';
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
    // The value type follows the schema. A date is a string here; a polygon
    // would be an array of [lat, lon] pairs, as in 06_publish_polygon.
    request.filter_json(std::string(R"({"date": ")") + kWantedDate + R"("})");

    Printer printer;
    std::cout << "listening to " << example::kEventType << " for date "
              << kWantedDate << '\n';
    aviso::Watch watch = client.watch(request, printer);
    return example::finish(watch, printer);
  });
}
