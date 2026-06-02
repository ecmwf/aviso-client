#pragma once

// Header-only C++ facade over the stable C ABI in `aviso.h`. It adds RAII
// handles, std::string and std::optional ergonomics, and a throwing
// `aviso::Error`. Compile it with your own C++17 (or later) toolchain over the
// prebuilt library; nothing here needs a Rust toolchain. The C header is the
// generated source of truth; this facade is hand-written and never generated.

#include "aviso.h"

#include <cstdint>
#include <cstdio>
#include <exception>
#include <map>
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

// Appends `value` to `out` as a JSON string literal, escaping the characters
// JSON requires. Bytes >= 0x20 (including UTF-8 continuation bytes) pass
// through unchanged; the C ABI only needs valid UTF-8, which the std::string
// already is on a conforming caller.
inline void append_json_string(std::string& out, const std::string& value) {
  out.push_back('"');
  for (const char ch : value) {
    switch (ch) {
      case '"':
        out += "\\\"";
        break;
      case '\\':
        out += "\\\\";
        break;
      case '\b':
        out += "\\b";
        break;
      case '\f':
        out += "\\f";
        break;
      case '\n':
        out += "\\n";
        break;
      case '\r':
        out += "\\r";
        break;
      case '\t':
        out += "\\t";
        break;
      default:
        if (static_cast<unsigned char>(ch) < 0x20) {
          char buffer[7];
          std::snprintf(buffer, sizeof(buffer), "\\u%04x",
                        static_cast<unsigned int>(static_cast<unsigned char>(ch)));
          out += buffer;
        } else {
          out.push_back(ch);
        }
    }
  }
  out.push_back('"');
}

// Serialises a string-to-string map as a compact JSON object, the wire form
// the C ABI's `identifier_json` argument expects.
inline std::string to_identifier_json(
    const std::map<std::string, std::string>& identifier) {
  std::string out = "{";
  bool first = true;
  for (const auto& entry : identifier) {
    if (!first) {
      out.push_back(',');
    }
    first = false;
    append_json_string(out, entry.first);
    out.push_back(':');
    append_json_string(out, entry.second);
  }
  out.push_back('}');
  return out;
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
    return take_string(aviso_client_schema(handle_.get()), "schema");
  }

  // Fetches the schema for one event type as a compact-JSON string, or throws
  // `aviso::Error`.
  [[nodiscard]] std::string schema_for(const std::string& event_type) {
    return take_string(aviso_client_schema_for(handle_.get(), event_type.c_str()),
                       "schema_for");
  }

  // Publishes a notification and returns the server's response as a
  // compact-JSON string, or throws `aviso::Error`. `identifier` is the
  // string-to-string identifier map; `payload`, when set, is a JSON string.
  [[nodiscard]] std::string notify(
      const std::string& event_type,
      const std::map<std::string, std::string>& identifier = {},
      const std::optional<std::string>& payload = std::nullopt) {
    std::string identifier_json;
    const char* identifier_ptr = nullptr;
    if (!identifier.empty()) {
      identifier_json = detail::to_identifier_json(identifier);
      identifier_ptr = identifier_json.c_str();
    }
    const char* payload_ptr = payload ? payload->c_str() : nullptr;
    return take_string(aviso_client_notify(handle_.get(), event_type.c_str(),
                                           identifier_ptr, payload_ptr),
                       "notify");
  }

  // Wipes every notification for one stream (operator-only), or throws
  // `aviso::Error`.
  void wipe_stream(const std::string& stream_name) {
    detail::check(
        detail::OutcomePtr(aviso_client_wipe_stream(handle_.get(), stream_name.c_str())));
  }

  // Wipes every stream (operator-only), or throws `aviso::Error`.
  void wipe_all() {
    detail::check(detail::OutcomePtr(aviso_client_wipe_all(handle_.get())));
  }

  // Deletes a single notification by its `<event_type>@<sequence>` id
  // (operator-only), or throws `aviso::Error`.
  void delete_notification(const std::string& notification_id) {
    detail::check(detail::OutcomePtr(
        aviso_client_delete_notification(handle_.get(), notification_id.c_str())));
  }

 private:
  friend class ClientBuilder;
  explicit Client(AvisoClient* handle) : handle_(handle) {}

  // Checks the outcome, takes its string success value, and returns it, or
  // throws `aviso::Error`. `what` names the verb for the diagnostic raised when
  // a success outcome unexpectedly carries no string.
  static std::string take_string(AvisoOutcome* raw, const char* what) {
    detail::OutcomePtr outcome = detail::check(detail::OutcomePtr(raw));
    detail::StringPtr text(aviso_outcome_take_string(outcome.get()));
    if (!text) {
      detail::throw_internal(std::string("aviso: ") + what +
                             " succeeded but returned no value");
    }
    return std::string(text.get());
  }

  detail::ClientPtr handle_;
};

// Fluent builder for a `Client`.
class ClientBuilder {
 public:
  explicit ClientBuilder(const std::string& base_url)
      : handle_(aviso_client_builder_new(base_url.c_str())) {
    if (!handle_) {
      detail::throw_internal("aviso: failed to allocate a client builder");
    }
  }

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
