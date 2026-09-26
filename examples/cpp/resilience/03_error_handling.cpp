// SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
// SPDX-License-Identifier: Apache-2.0

// What the errors look like, and which ones to act on.
//
// Every failure is an aviso::Error whose error() carries a kind, the HTTP
// status when there was one, and the server's request id for support. The
// kind is what you switch on. A transport error is worth a retry. A rejected
// credential is not. Invalid input is your bug.
//
// One thing that surprises people: a wrong password on a request is not an
// auth error. The server answers 401, so it arrives as an http error with
// that status. The auth kind covers the other credential problems: none
// available, one that would travel in the clear, or a watch refused even
// after the credential was refreshed. None of them is worth a retry.
//
// Some mistakes never reach the server. A WatchRequest is used up by the
// watch it starts; using it again throws invalid usage, which is a bug in the
// calling code, like invalid input.
//
// This example provokes six errors on purpose against a working server.
//
// Expect: six labelled errors, none of them fatal to the program. If one of
// the attempts succeeds, or fails with a different kind than this file
// says, the example exits 1, because then the server is not behaving the
// way this file describes.

#include "../common.hpp"

#include <cstdint>
#include <iostream>
#include <map>
#include <string>

namespace {

// Runs one call that should fail with a given kind (and HTTP status, when
// there is one). Returns true when it failed exactly that way, so a server
// that answers differently from what this file describes is noticed.
bool attempt(const char* what, void (*body)(aviso::Client&), aviso::Client& client,
             AvisoErrorKind expected_kind, std::uint16_t expected_status = 0) {
  std::cout << "-- " << what << '\n';
  try {
    body(client);
    std::cout << "   unexpectedly succeeded\n";
    return false;
  } catch (const aviso::Error& error) {
    example::report(error);
    // The kind is the thing to branch on. Everything else is for humans.
    const aviso::ErrorInfo& info = error.error();
    switch (info.kind) {
      case AvisoErrorKind_Transport:
        std::cout << "   -> retry later; the server or network is the problem\n";
        break;
      case AvisoErrorKind_Http:
        if (info.http_status == 401 || info.http_status == 403) {
          std::cout << "   -> the server rejected the credential; do not retry\n";
        } else if (info.http_status == 404) {
          std::cout << "   -> the event type is not on this server; check 01_schema\n";
        } else if (info.http_status == 400) {
          std::cout << "   -> the server rejected the request; fix the identifier\n";
        } else {
          std::cout << "   -> read the message; the server explained itself\n";
        }
        break;
      case AvisoErrorKind_Auth:
        std::cout << "   -> no usable credential; nothing was sent\n";
        break;
      case AvisoErrorKind_InvalidInput:
      case AvisoErrorKind_InvalidUsage:
        std::cout << "   -> a bug in the calling code\n";
        break;
      default:
        std::cout << "   -> something else; see kind_name() for the full list\n";
    }
    const bool as_expected =
        info.kind == expected_kind && (expected_status == 0 || info.http_status == expected_status);
    if (!as_expected) {
      std::cout << "   !! expected " << example::kind_name(expected_kind);
      if (expected_status != 0) {
        std::cout << " with HTTP " << expected_status;
      }
      std::cout << '\n';
    }
    return as_expected;
  }
}

}  // namespace

int main() {
  return example::run([] {
    aviso::Client client = example::connect();
    bool all_failed = true;

    all_failed &= attempt("event type that does not exist",
            [](aviso::Client& c) { static_cast<void>(c.schema_for("no-such-stream")); },
            client, AvisoErrorKind_Http, 404);

    all_failed &= attempt("identifier the schema rejects",
            [](aviso::Client& c) {
              const std::map<std::string, std::string> bad = {{"date", "yesterday"},
                                                             {"time", "0000"}};
              static_cast<void>(c.notify(example::kEventType, bad));
            },
            client, AvisoErrorKind_Http, 400);

    all_failed &= attempt("identifier JSON that is not an object",
            [](aviso::Client& c) {
              static_cast<void>(c.notify_json(example::kEventType, R"(["not","an","object"])"));
            },
            client, AvisoErrorKind_InvalidInput);

    // The same server, a credential it will not accept. Naming it after
    // from_environment() replaces whatever the file or environment supplied.
    aviso::ClientBuilder rejected = aviso::ClientBuilder::from_environment();
    rejected.basic_auth("nobody", "wrong");
    aviso::Client stranger = rejected.build();
    all_failed &= attempt("credential the server rejects",
            [](aviso::Client& c) {
              const std::map<std::string, std::string> id = {{"date", "20260101"},
                                                            {"time", "0000"}};
              static_cast<void>(c.notify(example::kEventType, id));
            },
            stranger, AvisoErrorKind_Http, 401);

    // A server that is not there. Building succeeds; the failure comes on
    // the first request.
    aviso::Client nowhere = aviso::ClientBuilder("http://127.0.0.1:1").build();
    all_failed &= attempt("server that is not listening",
            [](aviso::Client& c) { static_cast<void>(c.schema()); }, nowhere,
            AvisoErrorKind_Transport);
    // A request is used up by the watch it starts. Build a new one instead.
    all_failed &= attempt("watch request used twice",
            [](aviso::Client& c) {
              struct Ignore : aviso::NotificationHandler {
                bool on_notification(const aviso::Notification&) override { return false; }
              } handler;
              aviso::WatchRequest request(example::kEventType);
              { aviso::Watch first = c.watch(request, handler); }
              aviso::Watch second = c.watch(request, handler);
            },
            client, AvisoErrorKind_InvalidUsage);
    return all_failed ? 0 : 1;
  });
}
