/*
 * SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
 * SPDX-License-Identifier: Apache-2.0
 */

#pragma once

// Shared helpers for the C++ examples.
//
// Every example connects the same way, so the connection code lives here and
// each example shows only the thing it is about.
//
// `connect()` starts from the environment and the aviso config file. If you
// already use the `aviso` command on this machine, the examples pick up its
// server address and credentials with no setup. If not, set these:
//
//   AVISO_BASE_URL   the server, for example http://localhost:8000
//   AVISO_TOKEN      a bearer token
//   AVISO_USERNAME   with AVISO_PASSWORD, for HTTP Basic instead of a token
//
// The e2e stack in tests/e2e uses Basic. Its producer account is:
//
//   export AVISO_BASE_URL=http://localhost:8000
//   export AVISO_USERNAME=producer-user
//   export AVISO_PASSWORD=producer-pass

#include "aviso.hpp"

#include <cstdlib>
#include <exception>
#include <iostream>
#include <optional>
#include <string>

namespace example {

// The stream every example uses. The e2e stack ships it with `date` and
// `time` identifiers and requires the producer role to publish.
constexpr const char* kEventType = "test_event";

inline std::optional<std::string> env(const char* name) {
  const char* value = std::getenv(name);
  if (value == nullptr || *value == '\0') {
    return std::nullopt;
  }
  return std::string(value);
}

// Builds a client from whatever configuration this machine has.
//
// from_environment() reads the address from AVISO_BASE_URL, then from
// ~/.config/aviso/config.yaml when it exists, and finds a credential the way
// the `aviso` command does: AVISO_TOKEN first, then the Basic pair, then the
// file. A missing file sets nothing. `builder.describe()` says what it chose.
//
// Credentials from the environment are named here on purpose, even though
// from_environment() has already found them. A named credential may travel
// to a plain http address that is not loopback, which the CI stack is
// (http://aviso-server:8000). A found credential may not; the client refuses
// it at build(), so a mistyped host cannot send it in the clear. When you rely
// on the file alone, that protection applies to you too.
inline aviso::ClientBuilder configure() {
  aviso::ClientBuilder builder = aviso::ClientBuilder::from_environment();
  const auto token = env("AVISO_TOKEN");
  const auto username = env("AVISO_USERNAME");
  const auto password = env("AVISO_PASSWORD");
  if (token) {
    builder.bearer_auth(*token);
  } else if (username && password) {
    builder.basic_auth(*username, *password);
  }
  // Half a Basic pair is left alone. Discovery treats it as an error and
  // build() reports what is missing, before anything is sent.
  return builder;
}

inline aviso::Client connect() { return configure().build(); }

// A readable name for each error kind, for the examples' error output.
inline const char* kind_name(AvisoErrorKind kind) {
  switch (kind) {
    case AvisoErrorKind_Transport: return "transport";
    case AvisoErrorKind_Http: return "http";
    case AvisoErrorKind_Auth: return "auth";
    case AvisoErrorKind_Config: return "config";
    case AvisoErrorKind_Decode: return "decode";
    case AvisoErrorKind_HistoryGap: return "history gap";
    case AvisoErrorKind_MalformedEvent: return "malformed event";
    case AvisoErrorKind_StreamProtocol: return "stream protocol";
    case AvisoErrorKind_Trigger: return "trigger";
    case AvisoErrorKind_StateStore: return "state store";
    case AvisoErrorKind_InvalidInput: return "invalid input";
    case AvisoErrorKind_InvalidUsage: return "invalid usage";
    case AvisoErrorKind_Internal: return "internal";
    case AvisoErrorKind_Panic: return "panic";
    case AvisoErrorKind_Unknown: break;
  }
  return "unknown";
}

// Prints an error the way a user would want to read it: the kind first, then
// the message, then whatever the server told us.
inline void report(const aviso::ErrorInfo& info) {
  std::cerr << "aviso error [" << kind_name(info.kind) << "]";
  if (info.http_status != 0) {
    std::cerr << " (HTTP " << info.http_status << ")";
  }
  std::cerr << ": " << info.message << '\n';
  if (info.request_id) {
    std::cerr << "  request id: " << *info.request_id << '\n';
  }
}

inline void report(const aviso::Error& error) { report(error.error()); }

// A handler that remembers how the watch ended.
//
// on_end() is the only place a failed watch is reported, so a handler that
// ignores it turns every failure into a silent exit 0. Derive from this,
// override on_notification(), and hand the handler to finish() below.
class Handler : public aviso::NotificationHandler {
 public:
  void on_end(const std::optional<aviso::ErrorInfo>& error) override {
    failure_ = error;
  }

  // Set when the watch ended because of an error rather than because the
  // handler returned false.
  const std::optional<aviso::ErrorInfo>& failure() const { return failure_; }

 private:
  std::optional<aviso::ErrorInfo> failure_;
};

// The same for a merged watch (Client::watch_many): derive from this and
// override on_notification(), and on_error() to keep going past a failed
// watch.
class MultiHandler : public aviso::MultiNotificationHandler {
 public:
  void on_end(const std::optional<aviso::ErrorInfo>& error) override {
    failure_ = error;
  }

  const std::optional<aviso::ErrorInfo>& failure() const { return failure_; }

 private:
  std::optional<aviso::ErrorInfo> failure_;
};

// Waits for a watch to end and turns the outcome into an exit code. Works
// with Handler and MultiHandler alike.
template <typename H>
int finish(aviso::Watch& watch, const H& handler) {
  watch.wait();
  if (handler.failure()) {
    std::cerr << "watch ended with an error\n";
    report(*handler.failure());
    return 1;
  }
  return 0;
}

// Wraps a main body so every example ends the same way on failure.
template <typename F>
int run(F&& body) {
  try {
    return body();
  } catch (const aviso::Error& error) {
    report(error);
    return 1;
  } catch (const std::exception& error) {
    std::cerr << "error: " << error.what() << '\n';
    return 1;
  }
}

}  // namespace example
