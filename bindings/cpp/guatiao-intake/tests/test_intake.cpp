// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// guatiao-intake from C++: checking a form against its schema, its layout,
// the form a schema implies, visibility, paths into a value, and the flat
// `key -> text` store both ways. A counting allocator is the default, so
// every path is checked to free.

#include "support/guatiao_counting_default.hpp"  // first: it names the default allocator

#include <guatiao/intake.hpp>

#include "support/harness.hpp"

namespace {

using guatiao::item;
namespace intake = guatiao::intake;
using gt_test::counted;

#define CHECK_NONE_LEFT(what) \
  CHECK_MSG(counted().live() == 0, std::string(what) + ": " + std::to_string(counted().live()) + " left")

guatiao::value plain_schema() {
  return MUST(guatiao::try_make(item::map({
      {"type", "object"},
      {"properties", item::map({{"name", item::map({{"type", "string"}, {"title", "Name"}})}})},
      {"required", item::list({"name"})},
      {"additionalProperties", false},
  })));
}

void forms() {
  {
    guatiao::value schema = plain_schema();
    guatiao::value blank = MUST(guatiao::try_make(item::map({})));
    CHECK(intake::try_check(schema, blank).ok());
    guatiao::value bad = MUST(guatiao::try_make(item::map({{"sections", item::list({})},
                                                           {"fields", item::map({{"nope", item::map({})}})}})));
    auto refused = intake::try_check(schema, bad);
    CHECK(refused.code() == guatiao::status::bad_value);
    if (!refused.ok()) CHECK(refused.error().detail()["kind"].string_or("") == "unknown_field");

    guatiao::value sections = MUST(intake::try_layout(schema, blank));
    std::vector<std::string_view> fields;
    for (guatiao::view s : sections.as_list())
      for (guatiao::view f : s["fields"].as_list()) fields.push_back(f.string_or(""));
    CHECK(fields == std::vector<std::string_view>{"name"});

    guatiao::value values = MUST(guatiao::try_make(item::map({{"name", "ana"}})));
    CHECK(MUST(intake::try_is_visible(schema, blank, "name", values)));
    CHECK(intake::try_is_visible(schema, blank, "nope", values).code() == guatiao::status::not_found);
  }
  CHECK_NONE_LEFT("checking, laying out, visibility");
  {
    guatiao::value schema = MUST(guatiao::try_make(item::map({
        {"type", "object"},
        {"properties",
         item::map({{"host", item::map({{"type", "string"}, {"x-section", "net"}})},
                    {"port", item::map({{"type", "integer"}, {"x-section", "net"}})},
                    {"tls", item::map({{"type", "object"},
                                       {"properties", item::map({{"verify", item::map({{"type", "boolean"},
                                                                                        {"x-section", "trust"}})}})},
                                       {"additionalProperties", false}})},
                    {"label", item::map({{"type", "string"}})}})},
        {"additionalProperties", false},
    })));
    guatiao::value made = MUST(intake::try_for_schema(schema));
    CHECK(made["sections"].size() == 1 && made["sections"][0]["id"].string_or("") == "net");
    CHECK(made["fields"]["tls"]["form"]["sections"][0]["id"].string_or("") == "trust");
    CHECK(intake::try_check(schema, made).ok());
    guatiao::value blank = MUST(guatiao::try_make(item::map({})));
    CHECK(!MUST(intake::try_form_for(blank, schema, "tls")).is_absent());
    CHECK(MUST(intake::try_form_for(blank, schema, "label")).is_absent());
    CHECK(MUST(intake::try_form_for(blank, schema, "nonesuch")).is_absent());
  }
  CHECK_NONE_LEFT("forms made from a schema");
}

void paths() {
  guatiao::value v = MUST(guatiao::try_make(item::map({
      {"agent", item::list({item::map({{"name", "one"}}), item::map({{"name", "two"}})})},
      {"env", item::map({{"PATH", "/bin"}, {"a.b", "dotted"}, {"1", "keyed"}})},
  })));
  CHECK(intake::at(v, "agent[0].name").string_or("") == "one");
  CHECK(intake::at(v, "agent[1].name").string_or("") == "two");
  CHECK(intake::at(v, "env[PATH]").string_or("") == "/bin");
  CHECK(intake::at(v, "env[a.b]").string_or("") == "dotted");
  CHECK(intake::at(v, "env[\"1\"]").string_or("") == "keyed");
  CHECK(intake::at(v, "agent[9].name").is_absent());
  CHECK(intake::at(v, "nonesuch").is_absent());
  CHECK(intake::at(v, "agent[").is_absent());
}

guatiao::value flat_schema() {
  return MUST(guatiao::try_make(item::map({
      {"type", "object"},
      {"properties",
       item::map({
           {"auth", item::map({{"type", "object"},
                               {"x-variant-tag", "auth"},
                               {"oneOf", item::list({
                                             item::map({{"type", "object"},
                                                        {"properties", item::map({{"auth", item::map({{"const", "sso"}})}})},
                                                        {"required", item::list({"auth"})}}),
                                             item::map({{"type", "object"},
                                                        {"properties",
                                                         item::map({{"auth", item::map({{"const", "userpass"}})},
                                                                    {"username", item::map({{"type", "string"}})},
                                                                    {"password", item::map({{"type", "string"},
                                                                                            {"x-sensitive", true}})}})},
                                                        {"required", item::list({"auth"})}}),
                                         })}})},
           {"agent", item::map({{"type", "array"},
                                {"items", item::map({{"type", "object"},
                                                     {"properties", item::map({{"name", item::map({{"type", "string"}})}})}})}})},
           {"env", item::map({{"type", "object"}, {"additionalProperties", item::map({{"type", "string"}})}})},
           {"host", item::map({{"type", "string"}, {"title", "Host"}})},
       })},
  })));
}

void flat_store() {
  {
    guatiao::value schema = flat_schema();
    CHECK(intake::resolve(schema, "host")["title"].string_or("") == "Host");
    CHECK(intake::resolve(schema, "auth.password")["x-sensitive"].bool_or(false));
    CHECK(intake::resolve(schema, "nonesuch").is_absent());

    CHECK(MUST(intake::try_flat_keys(schema, "auth")) ==
          (std::vector<std::string>{"auth", "auth.username", "auth.password"}));
    CHECK(intake::try_flat_keys(schema, "nonesuch").code() == guatiao::status::wrong_kind);

    guatiao::value auth = MUST(guatiao::try_make(item::map({{"auth", "userpass"}, {"username", "ana"}, {"password", "pw"}})));
    guatiao::value flat = MUST(intake::try_flatten(schema, "auth", auth));
    CHECK(flat["auth"].string_or("") == "userpass");
    CHECK(flat["auth.username"].string_or("") == "ana");
    guatiao::value back = MUST(intake::try_unflatten(schema, "auth", flat));
    CHECK(back == auth);

    guatiao::value agents = MUST(guatiao::try_make(item::list({item::map({{"name", "a"}}), item::map({{"name", "b"}})})));
    guatiao::value flat_agents = MUST(intake::try_flatten(schema, "agent", agents));
    CHECK(flat_agents["agent[1].name"].string_or("") == "b");
    CHECK(MUST(intake::try_unflatten(schema, "agent", flat_agents)) == agents);

    guatiao::value wrong = MUST(guatiao::try_make(item::map({{"x", 1}})));
    CHECK(intake::try_flatten(schema, "agent", wrong).code() == guatiao::status::wrong_kind);
    guatiao::value empty = MUST(guatiao::try_make(item::map({})));
    CHECK(intake::try_unflatten(schema, "host", empty).code() == guatiao::status::wrong_kind);
  }
  CHECK_NONE_LEFT("the flat store both ways");
}

void provider_forms() {
  {
    guatiao::value schema = MUST(guatiao::try_make(item::map({
        {"type", "object"},
        {"properties", item::map({{"user", item::map({{"type", "string"}, {"x-section", "account"}})},
                                  {"token", item::map({{"type", "string"}, {"x-sensitive", true}})}})},
    })));
    guatiao::value declared = MUST(guatiao::try_make(
        item::map({{"sections", item::list({item::map({{"id", "account"}, {"title", "Account"}})})}})));
    guatiao::value meta = MUST(guatiao::try_make(item::map({{intake::provider_form_key, declared}, {"other", 1}})));

    // The one a provider declares.
    guatiao::value own = MUST(intake::try_provider_form(schema, meta));
    CHECK(own == declared);

    // None declared: the one its schema implies.
    guatiao::value made = MUST(intake::try_provider_form(schema, guatiao::view{}));
    CHECK(made["sections"].size() == 1 && made["sections"][0]["id"].string_or("") == "account");
    CHECK(made["fields"]["token"]["widget"].string_or("") == "password");
    guatiao::value other = MUST(guatiao::try_make(item::map({{"other", 1}})));
    CHECK(MUST(intake::try_provider_form(schema, other)) == made);

    // Nothing to show is not an error.
    CHECK(MUST(intake::try_provider_form(guatiao::view{}, guatiao::view{})).is_absent());
    CHECK(MUST(intake::try_provider_form(guatiao::view{}, other)).is_absent());

    // What is not a form is skipped, as reading the key by hand skips it.
    guatiao::value list_form = MUST(guatiao::try_make(
        item::map({{intake::provider_form_key, item::list({"not", "a", "form"})}})));
    CHECK(MUST(intake::try_provider_form(schema, list_form)) == made);
    guatiao::value text_form = MUST(guatiao::try_make(item::map({{intake::provider_form_key, "a form"}})));
    CHECK(MUST(intake::try_provider_form(guatiao::view{}, text_form)).is_absent());
  }
  CHECK_NONE_LEFT("a provider's form");
}

/// Each entry the registry's catalog lists is given the form it is shown with.
void catalog_entries_have_their_forms() {
  guatiao::registry reg = MUST(guatiao::registry::try_open("intake-catalog", "1.0"));
  guatiao::value report = MUST(reg.try_scan_dir(GUATIAO_CATALOG_DIR));
  CHECK(report["loaded"].size() == 1 && report["failed"].size() == 0);
  guatiao::value catalog = MUST(reg.try_catalog());
  CHECK(catalog.size() == 4);
  int seen = 0;
  for (guatiao::view entry : catalog.as_list()) {
    std::string_view id = entry["id"].string_or("");
    guatiao::value form = MUST(intake::try_provider_form(entry["config"], entry["meta"]));
    if (id == "catalog_configured") {
      // Its own form, under meta.
      CHECK(form["sections"][0]["title"].string_or("") == "Network");
      CHECK(form == entry["meta"][intake::provider_form_key]);
      CHECK(intake::try_check(entry["config"], form).ok());
      ++seen;
    } else if (id == "catalog_implied") {
      // None declared: the one its schema implies.
      CHECK(form["sections"][0]["id"].string_or("") == "account");
      CHECK(form["fields"]["token"]["widget"].string_or("") == "password");
      ++seen;
    } else {
      // Nothing to configure: nothing to show.
      CHECK(form.is_absent());
      ++seen;
    }
  }
  CHECK(seen == 4);
}

}  // namespace

int main() {
  catalog_entries_have_their_forms();
  provider_forms();
  forms();
  paths();
  flat_store();
  return gt_test::finish("intake");
}
