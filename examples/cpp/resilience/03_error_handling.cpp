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
// This example provokes five errors on purpose against a working server.
//
// Expect: five labelled errors, none of them fatal to the program.

#include "../common.hpp"

#include <iostream>
#include <map>
#include <string>

namespace {

void attempt(const char* what, void (*body)(aviso::Client&), aviso::Client& client) {
  std::cout << "-- " << what << '\n';
  try {
    body(client);
    std::cout << "   unexpectedly succeeded\n";
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
        std::cout << "   -> a bug in the calling code\n";
        break;
      default:
        std::cout << "   -> something else; see kind_name() for the full list\n";
    }
  }
}

}  // namespace

int main() {
  return example::run([] {
    aviso::Client client = example::connect();

    attempt("event type that does not exist",
            [](aviso::Client& c) { static_cast<void>(c.schema_for("no-such-stream")); },
            client);

    attempt("identifier the schema rejects",
            [](aviso::Client& c) {
              const std::map<std::string, std::string> bad = {{"date", "yesterday"},
                                                             {"time", "0000"}};
              static_cast<void>(c.notify(example::kEventType, bad));
            },
            client);

    attempt("identifier JSON that is not an object",
            [](aviso::Client& c) {
              static_cast<void>(c.notify_json(example::kEventType, R"(["not","an","object"])"));
            },
            client);

    // The same server, a credential it will not accept. Naming it after
    // from_file() replaces whatever the file or environment supplied.
    aviso::ClientBuilder rejected = aviso::ClientBuilder::from_file();
    if (const auto url = example::env("AVISO_BASE_URL")) {
      rejected.base_url(*url);
    }
    rejected.basic_auth("nobody", "wrong");
    aviso::Client stranger = rejected.build();
    attempt("credential the server rejects",
            [](aviso::Client& c) {
              const std::map<std::string, std::string> id = {{"date", "20260101"},
                                                            {"time", "0000"}};
              static_cast<void>(c.notify(example::kEventType, id));
            },
            stranger);

    // A server that is not there. Building succeeds; the failure comes on
    // the first request.
    aviso::Client nowhere = aviso::ClientBuilder("http://127.0.0.1:1").build();
    attempt("server that is not listening",
            [](aviso::Client& c) { static_cast<void>(c.schema()); }, nowhere);
    return 0;
  });
}
