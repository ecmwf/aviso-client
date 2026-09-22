// SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
// SPDX-License-Identifier: Apache-2.0

// Consumer smoke for the cargo-c install layout: include the facade through
// the installed `aviso_ffi` pkg-config package and call a no-server verb. It
// proves a clean consumer compiles, links, and runs against the staged
// `.a/.so/.dylib` + `.pc` + headers, with no Rust toolchain and no in-tree paths.

#include "aviso.hpp"

#include <cstdlib>
#include <iostream>
#include <string>

int main() {
  std::cout << "aviso version: " << aviso::version() << '\n';

  // Point every credential source at somewhere that holds nothing, so the
  // result does not depend on what the machine running this has on disk.
  for (const char* name :
       {"AVISO_TOKEN", "AVISO_USERNAME", "AVISO_PASSWORD"}) {
    unsetenv(name);
  }
  setenv("AVISO_CLIENT_CONFIG_FILE", "/nonexistent/aviso/config.yaml", 1);
  setenv("AVISO_CREDENTIALS_FILE", "/nonexistent/aviso/credentials.yaml", 1);

  // Discovery therefore finds nothing, so the explicit token is what the
  // client ends up with. Calling both also makes the linker check that the
  // packaged library exports both entry points.
  aviso::Client client = aviso::ClientBuilder("http://127.0.0.1:1")
                             .bearer_auth("smoke-token")
                             .discover_auth()
                             .build();

  // The config file was pointed at a missing path above, so from_file()
  // sets nothing; the address supplied afterwards makes it buildable. This
  // also links both from_file entry points.
  aviso::Client from_file = aviso::ClientBuilder::from_file()
                                .base_url("http://127.0.0.1:1")
                                .bearer_auth("smoke-token")
                                .build();
  static_cast<void>(from_file);

  // The path overload is inline, so its entry point is only linked when it
  // is used. This smoke also runs against a stub library that cannot know
  // whether a file exists, so only the link is checked here; the behaviour
  // is covered by the tests that run against the real library.
  static_cast<void>(
      aviso::ClientBuilder::from_file("/nonexistent/aviso/config.yaml"));
  const std::string malformed_identifier = R"(["not-an-object"])";

  bool blocking_rejected = false;
  try {
    static_cast<void>(
        client.notify_json("observations", malformed_identifier));
  } catch (const aviso::Error&) {
    blocking_rejected = true;
  }

  bool async_rejected = false;
  try {
    client.notify_json_async("observations", malformed_identifier).get();
  } catch (const aviso::Error&) {
    async_rejected = true;
  }

  if (!blocking_rejected || !async_rejected) {
    std::cerr << "JSON-valued notify methods accepted a non-object identifier\n";
    return 1;
  }
  return 0;
}
