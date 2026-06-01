#pragma once

// Header-only C++ facade over the stable C ABI in `aviso.h`. It adds RAII
// handles, std::string and std::optional ergonomics, and a throwing
// `aviso::Error`. Compile it with your own C++17 (or later) toolchain over the
// prebuilt library; nothing here needs a Rust toolchain. The C header is the
// generated source of truth; this facade is hand-written and never generated.

#include "aviso.h"

#include <cstdint>
#include <exception>
#include <memory>
#include <optional>
#include <string>
#include <utility>

namespace aviso {

// An owned copy of a structured error, safe to keep after the originating
// outcome has been freed.
struct ErrorInfo {
  AvisoErrorKind kind = AvisoErrorKind_Unknown;
  std::uint16_t http_status = 0;
  std::string message;
  std::optional<std::string> request_id;
  std::optional<std::string> trigger_kind;
  std::optional<std::string> error_kind;
};

// Thrown by the facade when a call fails. `what()` is the human-readable
// message; `error()` exposes the full structured detail.
class Error : public std::exception {
 public:
  explicit Error(ErrorInfo info) : info_(std::move(info)) {}

  [[nodiscard]] const char* what() const noexcept override {
    return info_.message.c_str();
  }

  [[nodiscard]] const ErrorInfo& error() const noexcept { return info_; }

 private:
  ErrorInfo info_;
};

namespace detail {

inline std::optional<std::string> optional_string(const char* value) {
  if (value == nullptr) {
    return std::nullopt;
  }
  return std::string(value);
}

[[noreturn]] inline void throw_internal(std::string message) {
  ErrorInfo info;
  info.kind = AvisoErrorKind_Internal;
  info.message = std::move(message);
  throw Error(std::move(info));
}

inline ErrorInfo to_error_info(const AvisoError* error) {
  ErrorInfo info;
  if (error == nullptr) {
    info.kind = AvisoErrorKind_Internal;
    info.message = "aviso: outcome reported a failure but carried no error";
    return info;
  }
  info.kind = error->kind;
  info.http_status = error->http_status;
  info.message =
      error->message != nullptr ? std::string(error->message) : std::string();
  info.request_id = optional_string(error->request_id);
  info.trigger_kind = optional_string(error->trigger_kind);
  info.error_kind = optional_string(error->error_kind);
  return info;
}

struct OutcomeDeleter {
  void operator()(AvisoOutcome* outcome) const noexcept {
    aviso_outcome_free(outcome);
  }
};
using OutcomePtr = std::unique_ptr<AvisoOutcome, OutcomeDeleter>;

struct StringDeleter {
  void operator()(char* text) const noexcept { aviso_string_free(text); }
};
using StringPtr = std::unique_ptr<char, StringDeleter>;

struct ClientDeleter {
  void operator()(AvisoClient* client) const noexcept {
    aviso_client_free(client);
  }
};
using ClientPtr = std::unique_ptr<AvisoClient, ClientDeleter>;

struct BuilderDeleter {
  void operator()(AvisoClientBuilder* builder) const noexcept {
    aviso_client_builder_free(builder);
  }
};
using BuilderPtr = std::unique_ptr<AvisoClientBuilder, BuilderDeleter>;

// Throws `aviso::Error` when the outcome carries an error; returns it
// unchanged otherwise so the caller can take a success value out of it.
inline OutcomePtr check(OutcomePtr outcome) {
  if (!outcome) {
    throw_internal("aviso: received a null outcome");
  }
  if (!aviso_outcome_is_ok(outcome.get())) {
    throw Error(to_error_info(aviso_outcome_error(outcome.get())));
  }
  return outcome;
}

}  // namespace detail

// Version of the underlying library.
[[nodiscard]] inline std::string version() {
  return std::string(aviso_version());
}

class ClientBuilder;

// An RAII client. Move-only; the underlying handle is freed on destruction.
class Client {
 public:
  // Fetches the schema catalog as a compact-JSON string, or throws
  // `aviso::Error`.
  [[nodiscard]] std::string schema() {
    detail::OutcomePtr outcome =
        detail::check(detail::OutcomePtr(aviso_client_schema(handle_.get())));
    detail::StringPtr text(aviso_outcome_take_string(outcome.get()));
    if (!text) {
      detail::throw_internal("aviso: schema succeeded but returned no value");
    }
    return std::string(text.get());
  }

 private:
  friend class ClientBuilder;
  explicit Client(AvisoClient* handle) : handle_(handle) {}
  detail::ClientPtr handle_;
};

// Fluent builder for a `Client`.
class ClientBuilder {
 public:
  explicit ClientBuilder(const std::string& base_url)
      : handle_(aviso_client_builder_new(base_url.c_str())) {}

  ClientBuilder& basic_auth(const std::string& username,
                            const std::string& password) {
    aviso_client_builder_basic_auth(handle_.get(), username.c_str(),
                                    password.c_str());
    return *this;
  }

  // Builds the client, consuming this builder's handle, or throws
  // `aviso::Error`.
  [[nodiscard]] Client build() {
    AvisoClientBuilder* raw = handle_.release();
    detail::OutcomePtr outcome =
        detail::check(detail::OutcomePtr(aviso_client_builder_build(&raw)));
    AvisoClient* client = aviso_outcome_take_client(outcome.get());
    if (client == nullptr) {
      detail::throw_internal("aviso: build succeeded but returned no client");
    }
    return Client(client);
  }

 private:
  detail::BuilderPtr handle_;
};

}  // namespace aviso
