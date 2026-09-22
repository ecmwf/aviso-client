// SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
// SPDX-License-Identifier: Apache-2.0

// Read history and stop, without waiting for anything new.
//
// Sometimes you want a batch, not a feed: everything since a date, or since
// a sequence, and then exit. replay_from_* gives you that. The watch ends by
// itself once history is exhausted, so the handler returns true throughout
// and wait() still returns.
//
// One thing to know: a server caps how much one replay may return. On the
// e2e stack the cap is 50. When you hit it the watch ends with a history
// gap error that names the cap, and the notifications you did get are all
// in order. The fix is to ask again from the last sequence you saw. This
// example does that in a loop, so it reads everything however long the
// history is.
//
// Expect: every notification kept since the date below, in batches if the
// history is long, then a count, then exit. Publish a few first if the
// list is empty.

#include "../common.hpp"

#include <atomic>
#include <cstdint>
#include <iostream>
#include <optional>

namespace {

// Far enough back to cover everything the e2e stack has kept. Real code
// would compute this from the clock or from the last run.
constexpr const char* kSince = "2026-01-01T00:00:00Z";

class Counter : public example::Handler {
 public:
  bool on_notification(const aviso::Notification& n) override {
    std::cout << "#" << n.sequence() << "  " << n.identifier_json() << '\n';
    last_ = n.sequence();
    count_.fetch_add(1);
    return true;  // never stop early; the replay ends on its own
  }

  int count() const { return count_.load(); }
  std::optional<std::uint64_t> last() const { return last_; }

 private:
  std::atomic<int> count_{0};
  std::optional<std::uint64_t> last_;
};

// True when the watch ended with a history gap after delivering something.
// The cap is the usual cause. A jump in sequence numbers lands here too, and
// asking again from the last sequence we saw is the right answer to both.
bool stopped_short(const Counter& counter) {
  return counter.failure() && counter.failure()->kind == AvisoErrorKind_HistoryGap &&
         counter.last().has_value();
}

}  // namespace

int main() {
  return example::run([] {
    aviso::Client client = example::connect();

    int total = 0;
    std::optional<std::uint64_t> resume_after;
    for (;;) {
      aviso::WatchRequest request(example::kEventType);
      if (resume_after) {
        // The rest of the history, after the last one we saw.
        request.replay_from_sequence(*resume_after);
      } else {
        request.replay_from_date(kSince);
      }

      Counter counter;
      aviso::Watch watch = client.watch(request, counter);
      watch.wait();
      total += counter.count();

      if (stopped_short(counter)) {
        std::cout << "(server stopped short; continuing after #" << *counter.last() << ")\n";
        resume_after = counter.last();
        continue;
      }
      if (counter.failure()) {
        // Any other failure is a real one.
        example::report(*counter.failure());
        return 1;
      }
      break;
    }

    std::cout << total << " notification(s) replayed\n";
    return 0;
  });
}
