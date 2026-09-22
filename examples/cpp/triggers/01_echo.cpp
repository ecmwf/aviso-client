// SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
// SPDX-License-Identifier: Apache-2.0

// The simplest trigger: print each notification as it arrives.
//
// A trigger is something the client does for you on every notification,
// alongside (or instead of) your handler. echo() writes the notification to
// stdout as one line of JSON. It is the right first trigger to try, because
// you can see exactly what the others will receive.
//
// The handler here does nothing but count, to show that triggers and the
// handler are independent: the trigger fires whether or not you look.
//
// Expect: three JSON lines from the trigger, then exit.

#include "../common.hpp"

#include <atomic>

namespace {

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
    request.add_trigger(aviso::Trigger::echo());

    CountOnly handler;
    aviso::Watch watch = client.watch(request, handler);
    return example::finish(watch, handler);
  });
}
