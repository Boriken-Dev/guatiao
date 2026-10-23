// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// A plugin host written in C++.
//
// Watches the directories it is given for libraries declaring the greeter
// kind, whatever language wrote them, and calls every greeter through the
// kind's table: a map back, a map lent and changed in place, a map handed
// over, an object lent for one call, an object handed in and one handed
// back. A provider building instances is given a configuration its schema
// accepts, after one it refuses. With `--python-source BRIDGE
// --python-dir DIR` the guatiao-py bridge serves the Python greeters
// described under DIR too. Every library is unloaded where it agrees, and
// retired where it does not.
//
//     cpp_host DIR [DIR ...] [--python-source BRIDGE --python-dir DIR ...]

#include "greeter_kind.hpp"

#include <cstdio>

namespace gk = greeter_kind;
using guatiao::item;

/// What a conversation tells: kept.
struct ear {
  std::vector<std::string> heard_so_far;
  void heard(std::string_view what) { heard_so_far.emplace_back(what); }
};

std::string joined(const std::vector<std::string>& xs) {
  std::string s;
  for (const std::string& x : xs) s += (s.empty() ? "" : ", ") + x;
  return "[" + s + "]";
}

void exercise(const guatiao::remote<gk::greeter>& g) {
  std::printf("  greet: %s\n", std::string(g.greet("ana")["greeting"].string_or("?")).c_str());
  auto refused = g.try_greet("");
  if (!refused.ok()) std::printf("  greet refused: %s\n", refused.error().what());
  if (g.has_shout()) std::printf("  shout: %s\n", g.shout("ana").c_str());
  if (g.has_check()) {
    guatiao::map config{{"name", "bo"}};
    bool usable = g.check(config.ref());
    std::printf("  check: %s, lent map now has %zu entries\n", usable ? "true" : "false", config.size());
  }
  if (g.has_greet_from()) {
    std::printf("  greet_from: %s\n",
                std::string(g.greet_from(guatiao::map{{"name", "cy"}})["greeting"].string_or("?")).c_str());
  }
  if (g.has_greet_into()) {
    ear lent;
    g.greet_into("dee", lent);
    std::printf("  greet_into: the lent listener heard %s\n", joined(lent.heard_so_far).c_str());
  }
  if (g.has_start()) {
    auto heard = std::make_unique<ear>();
    ear* told = heard.get();
    guatiao::object<gk::conversation> chat = g.start(guatiao::object<gk::listener>::of(std::move(heard)));
    chat.say("hi");
    chat.say("again");
    std::uint8_t transcript[64];
    std::int64_t n = chat.read(transcript);
    std::printf("  conversation: %lld turns, %lld bytes read, listener heard %s\n",
                static_cast<long long>(chat.turns()), static_cast<long long>(n), joined(told->heard_so_far).c_str());
  }
}

int main(int argc, char** argv) {
  std::vector<std::string> dirs, python_dirs;
  std::string bridge;
  for (int i = 1; i < argc; ++i) {
    std::string arg = argv[i];
    if (arg == "--python-source" && i + 1 < argc) bridge = argv[++i];
    else if (arg == "--python-dir" && i + 1 < argc) python_dirs.emplace_back(argv[++i]);
    else dirs.push_back(arg);
  }
  if (dirs.empty() && bridge.empty()) {
    std::fprintf(stderr, "usage: cpp_host DIR [DIR ...] [--python-source BRIDGE --python-dir DIR ...]\n");
    return 2;
  }

  try {
    guatiao::registry reg("cpp-host", "1.0");
    for (const std::string& dir : dirs) reg.watch_dir(dir, guatiao::rules({"kind=greeter"}));
    if (!bridge.empty()) {
      std::vector<item> listed(python_dirs.begin(), python_dirs.end());
      reg.add_source_file(bridge, guatiao::make(item::map({{"dirs", item::list_from(listed)}})));
    }
    guatiao::value changed = reg.refresh();
    for (guatiao::view one : changed["loaded"].as_list())
      std::printf("loaded %s\n", std::string(one["key"].string_or("?")).c_str());
    for (guatiao::view one : changed["refused"].as_list())
      std::printf("refused %s: %s\n", std::string(one["id"].string_or("")).c_str(),
                  std::string(one["why"].string_or("")).c_str());

    guatiao::value providers = reg.providers("greeter");
    for (guatiao::view provider : providers.as_list()) {
      std::string key(provider["key"].string_or(""));
      std::printf("\n%s (%s %s)\n", key.c_str(), std::string(provider["library"].string_or("")).c_str(),
                  std::string(provider["version"].string_or("")).c_str());
      auto bound = gk::greeter::try_bind(reg, key);
      if (!bound) {
        std::printf("  not this greeter kind: %s\n", bound.error().what());
        continue;
      }
      if (!provider["has_config"].bool_or(false)) {
        exercise(*bound);
        continue;
      }
      // Built from a configuration: one its schema refuses, then one it
      // accepts. The registry checks it before the provider sees it.
      auto refused = reg.try_create(key, guatiao::make(item::map({})));
      if (!refused) std::printf("  an empty configuration is refused: %s\n", refused.error().what());
      guatiao::instance inst = reg.create(key, guatiao::make(item::map({{"prefix", "hey"}})));
      std::printf("  instance from {\"prefix\": \"hey\"}\n");
      exercise(gk::greeter::bind(inst));
    }

    // Unmapped where the library agrees; one with no say in it is retired
    // instead, and stays mapped.
    std::printf("\n");
    guatiao::value libraries = reg.libraries();
    for (guatiao::view library : libraries.as_list()) {
      std::string key(library["key"].string_or(""));
      auto unloaded = reg.try_unload(key);
      if (unloaded) {
        std::printf("unloaded %s\n", key.c_str());
      } else {
        (void)reg.try_retire(key);
        std::printf("retired %s, still mapped: unload said %s\n", key.c_str(), guatiao::status_name(unloaded.code()));
      }
    }
  } catch (const guatiao::error& e) {
    std::fprintf(stderr, "cpp_host: %s\n", e.what());
    return 1;
  }
  return 0;
}
