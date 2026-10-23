// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Doubles to number text and back where <charconv> has no floating point
// (built with GUATIAO_CPP_NO_FLOAT_CHARCONV, so every compiler takes that
// path here): the fewest digits that read back, written with '.', and
// read back exactly, under the "C" locale and under one writing ','.

#include <guatiao/guatiao.hpp>

#include <clocale>

#include "support/harness.hpp"

namespace {

void doubles(const char* where) {
  const double cases[] = {0.1, 0.25, -2.5e-3, 1e300, 123456789012.0, 5e-324, 1.7976931348623157e308, 100.0};
  for (double d : cases) {
    auto v = guatiao::try_make(guatiao::item(d));
    CHECK_MSG(v.ok(), std::string(where) + ": a double builds");
    if (!v.ok()) continue;
    std::string text(v->number_text().value_or(""));
    CHECK_MSG(text.find(',') == std::string::npos, std::string(where) + ": written with a point: " + text);
    CHECK_MSG(v->as<double>() == d, std::string(where) + ": read back exactly: " + text);
  }
  CHECK_MSG(guatiao::try_make(guatiao::item(0.1))->number_text() == std::string_view("0.1"),
            std::string(where) + ": 0.1 is 0.1");
  auto parsed = guatiao::try_make(guatiao::item::number("1.5"));
  CHECK_MSG(parsed.ok() && parsed->as<double>() == 1.5, std::string(where) + ": 1.5 reads as 1.5");
}

}  // namespace

int main() {
  doubles("C locale");
  const char* commas[] = {"de_DE.UTF-8", "de_DE.utf8", "de-DE", "German_Germany.1252", "fr_FR.UTF-8"};
  const char* found = nullptr;
  for (const char* name : commas) {
    if (std::setlocale(LC_NUMERIC, name)) {
      found = name;
      break;
    }
  }
  if (found && std::localeconv()->decimal_point[0] == ',') {
    doubles(found);
  } else {
    gt_test::skip_machine("no locale writing a decimal comma is installed");
  }
  std::setlocale(LC_NUMERIC, "C");
  return gt_test::finish("number text");
}
