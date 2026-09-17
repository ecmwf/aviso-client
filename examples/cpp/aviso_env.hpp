/*
 * SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
 * SPDX-License-Identifier: Apache-2.0
 */

#pragma once

// Shared helper for the examples. Connection settings come from the
// environment, never the command line, so credentials never land in shell
// history or `ps` output:
//
//   AVISO_BASE_URL   server base URL (default http://localhost:8000)
//   AVISO_USERNAME   HTTP Basic username (optional)
//   AVISO_PASSWORD   HTTP Basic password (optional)
//
// When both AVISO_USERNAME and AVISO_PASSWORD are set the client uses HTTP
// Basic auth; otherwise it connects anonymously.

#include "aviso.hpp"

#include <cstdlib>
#include <optional>
#include <string>

namespace example {

inline std::optional<std::string> env(const char* name) {
  const char* value = std::getenv(name);
  if (value == nullptr) {
    return std::nullopt;
  }
  return std::string(value);
}

inline std::string base_url() {
  return env("AVISO_BASE_URL").value_or("http://localhost:8000");
}

inline aviso::Client connect() {
  aviso::ClientBuilder builder(base_url());
  const std::optional<std::string> username = env("AVISO_USERNAME");
  const std::optional<std::string> password = env("AVISO_PASSWORD");
  if (username && password) {
    builder.basic_auth(*username, *password);
  }
  return builder.build();
}

}  // namespace example
