// SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
// SPDX-License-Identifier: Apache-2.0

// Publish to a stream with a spatial identifier and a required payload.
//
// notify() takes a map of string values, which covers dates, times and
// names. When an identifier field has structure, such as a list of points,
// notify_json() takes the whole identifier as JSON and lets you write any
// shape the schema accepts. Writing the identifier as JSON also makes the
// request easy to read next to the schema.
//
// The e2e stack's test_polygon stream requires a payload and has a polygon
// field. Its server takes the polygon as one string of "lat,lon,lat,lon,..."
// pairs, closed by repeating the first point. Newer servers take the same
// polygon as [[lat, lon], ...] arrays; swap the line below and nothing else
// changes, which is the point of using notify_json() here.
//
// Expect: the server's response. The Python example basics/02_listen shows
// a listener with a polygon filter on this stream.

#include "../common.hpp"

#include <iostream>
#include <string>

int main() {
  return example::run([] {
    aviso::Client client = example::connect();

    const std::string identifier = R"({
      "date": "20260101",
      "time": "0000",
      "polygon": "0,0,1,0,1,1,0,0"
    })";
    // On a newer server:
    //   "polygon": [[0, 0], [1, 0], [1, 1], [0, 0]]

    const std::string payload = R"({"source": "cpp-example", "area": "unit"})";

    const std::string response = client.notify_json("test_polygon", identifier, payload);
    std::cout << "published: " << response << '\n';
    return 0;
  });
}
