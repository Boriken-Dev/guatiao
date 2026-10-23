// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Uses each package once: a configuration read from JSON (guatiao-serde),
// checked against its schema (guatiao), shown as a form's fields in order
// (the unsectioned ones first) and stored flat (guatiao-intake), and
// written back as JSON.

#include <guatiao/intake.hpp>
#include <guatiao/serde.hpp>

#include <cstdio>

int main() {
  namespace serde = guatiao::serde;
  namespace intake = guatiao::intake;
  try {
    guatiao::value schema = serde::parse(serde::format::json, R"({
      "type": "object",
      "properties": {
        "host": {"type": "string", "x-section": "net"},
        "port": {"type": "integer", "minimum": 1, "x-section": "net"},
        "label": {"type": "string"}
      },
      "required": ["host"],
      "additionalProperties": false
    })");
    guatiao::value config = serde::parse(serde::format::json, R"({"host": "10.0.0.1", "port": 5900})");
    guatiao::validate(schema, config);

    guatiao::value form = intake::for_schema(schema);
    guatiao::value sections = intake::layout(schema, form);
    std::string fields;
    for (guatiao::view section : sections.as_list())
      for (guatiao::view field : section["fields"].as_list()) fields += std::string(field.string_or("?")) + " ";

    guatiao::value flat = intake::flatten(schema, "port", config["port"]);
    std::string text = serde::emit(serde::format::json, config);
    std::printf("fields: %s\nport stored as: %s\nconfig: %s\n", fields.c_str(),
                std::string(flat["port"].string_or("?")).c_str(), text.c_str());
    if (fields != "label host port " || flat["port"].string_or("") != "5900" ||
        text != R"({"host":"10.0.0.1","port":5900})") {
      std::printf("consumer: unexpected answer\n");
      return 1;
    }
    auto refused = guatiao::try_validate(schema, serde::parse(serde::format::json, R"({"port": 0})"));
    if (refused.ok()) {
      std::printf("consumer: a configuration without a host was accepted\n");
      return 1;
    }
    std::printf("consumer: ok (refused: %s)\n", refused.error().what());
    return 0;
  } catch (const guatiao::error& e) {
    std::printf("consumer: %s\n", e.what());
    return 1;
  }
}
