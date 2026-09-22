// SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
// SPDX-License-Identifier: Apache-2.0

// Pick up where you left off.
//
// Every notification carries a sequence number. If your process stops, the
// last sequence you handled is all you need to resume without a gap: ask for
// everything after it, then keep listening live. The server replays history
// first, then switches to live delivery on the same connection.
//
// This example remembers the sequence in a file between runs, which is the
// simplest possible state store. Run it, publish a few notifications while
// it is stopped, run it again, and the ones you missed arrive first.
//
// Expect: on the first run, live notifications. On later runs, the missed
// ones replayed, then live ones, with no gap. A notification published just
// as you reconnect can arrive twice, once from replay and once live. That is
// what at-least-once means; keep the sequence and skip what you have seen.

#include "../common.hpp"

#include <atomic>
#include <cstdint>
#include <fstream>
#include <iostream>
#include <optional>
#include <string>

namespace {

constexpr const char* kStateFile = "resume.seq";
constexpr int kStopAfter = 3;

std::optional<std::uint64_t> load_sequence() {
  std::ifstream in(kStateFile);
  std::uint64_t sequence = 0;
  if (in >> sequence) {
    return sequence;
  }
  return std::nullopt;
}

// True only when the position is on disk. A checkpoint that did not land
// would make the next run skip notifications, which is the one thing this
// example promises not to do.
bool save_sequence(std::uint64_t sequence) {
  std::ofstream out(kStateFile, std::ios::trunc);
  out << sequence << '\n';
  out.flush();
  return out.good();
}

class Resuming : public example::Handler {
 public:
  bool on_notification(const aviso::Notification& n) override {
    std::cout << "#" << n.sequence() << "  " << n.identifier_json() << '\n';
    // Record progress only after handling, so a crash mid-handler replays
    // this notification rather than skipping it. At-least-once, not at-most.
    if (!save_sequence(n.sequence())) {
      std::cerr << "could not write " << kStateFile << "; stopping\n";
      save_failed_ = true;
      return false;
    }
    return seen_.fetch_add(1) + 1 < kStopAfter;
  }

  bool save_failed() const { return save_failed_; }

 private:
  std::atomic<int> seen_{0};
  bool save_failed_ = false;
};

}  // namespace

int main() {
  return example::run([] {
    aviso::Client client = example::connect();
    aviso::WatchRequest request(example::kEventType);

    if (const auto last = load_sequence()) {
      // "Everything after this, then live." The server fills the gap.
      request.watch_from_sequence(*last);
      std::cout << "resuming after #" << *last << '\n';
    } else {
      std::cout << "no saved position; starting live\n";
    }

    Resuming handler;
    aviso::Watch watch = client.watch(request, handler);
    int rc = example::finish(watch, handler);
    if (handler.save_failed()) {
      rc = 1;
    } else if (load_sequence()) {
      std::cout << "position saved in " << kStateFile << "; run again to resume\n";
    }
    return rc;
  });
}
