// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// A JSON reader for the shared vector files, written over the binding's
// own builders so the core's suite needs no serde: numbers keep their
// exact text, `\u` escapes (surrogate pairs and NUL included) become
// UTF-8, objects keep their order.

#ifndef GT_TEST_JSON_HPP
#define GT_TEST_JSON_HPP

#include <guatiao/guatiao.hpp>

#include <string>
#include <string_view>

namespace gt_test {

class json_reader {
 public:
  explicit json_reader(std::string_view text) : t_(text) {}

  /// The document as an item, or false with the offset in `at`.
  bool read(guatiao::item& out) {
    ws();
    if (!value(out)) return false;
    ws();
    return i_ == t_.size();
  }
  std::size_t at() const { return i_; }

 private:
  void ws() {
    while (i_ < t_.size() && (t_[i_] == ' ' || t_[i_] == '\n' || t_[i_] == '\r' || t_[i_] == '\t')) ++i_;
  }
  bool lit(std::string_view w) {
    if (t_.substr(i_, w.size()) != w) return false;
    i_ += w.size();
    return true;
  }
  bool value(guatiao::item& out) {
    if (i_ >= t_.size()) return false;
    char c = t_[i_];
    if (c == '{') return object(out);
    if (c == '[') return array(out);
    if (c == '"') {
      std::string s;
      if (!string(s)) return false;
      out = guatiao::item(std::move(s));
      return true;
    }
    if (lit("true")) return out = guatiao::item(true), true;
    if (lit("false")) return out = guatiao::item(false), true;
    if (lit("null")) return out = guatiao::item(nullptr), true;
    std::size_t start = i_;
    while (i_ < t_.size() && std::string_view("+-0123456789.eE").find(t_[i_]) != std::string_view::npos) ++i_;
    if (i_ == start) return false;
    out = guatiao::item::number(t_.substr(start, i_ - start));
    return true;
  }
  bool object(guatiao::item& out) {
    ++i_;
    std::vector<std::pair<std::string, guatiao::item>> items;
    ws();
    if (i_ < t_.size() && t_[i_] == '}') {
      ++i_;
      out = guatiao::item::map_from(std::move(items));
      return true;
    }
    for (;;) {
      ws();
      std::string key;
      if (i_ >= t_.size() || t_[i_] != '"' || !string(key)) return false;
      ws();
      if (!lit(":")) return false;
      ws();
      guatiao::item v;
      if (!value(v)) return false;
      items.emplace_back(std::move(key), std::move(v));
      ws();
      if (lit(",")) continue;
      if (lit("}")) break;
      return false;
    }
    out = guatiao::item::map_from(std::move(items));
    return true;
  }
  bool array(guatiao::item& out) {
    ++i_;
    std::vector<guatiao::item> items;
    ws();
    if (lit("]")) {
      out = guatiao::item::list_from(std::move(items));
      return true;
    }
    for (;;) {
      ws();
      guatiao::item v;
      if (!value(v)) return false;
      items.push_back(std::move(v));
      ws();
      if (lit(",")) continue;
      if (lit("]")) break;
      return false;
    }
    out = guatiao::item::list_from(std::move(items));
    return true;
  }
  static void utf8(std::string& s, unsigned cp) {
    if (cp < 0x80) {
      s.push_back(static_cast<char>(cp));
    } else if (cp < 0x800) {
      s.push_back(static_cast<char>(0xC0 | (cp >> 6)));
      s.push_back(static_cast<char>(0x80 | (cp & 0x3F)));
    } else if (cp < 0x10000) {
      s.push_back(static_cast<char>(0xE0 | (cp >> 12)));
      s.push_back(static_cast<char>(0x80 | ((cp >> 6) & 0x3F)));
      s.push_back(static_cast<char>(0x80 | (cp & 0x3F)));
    } else {
      s.push_back(static_cast<char>(0xF0 | (cp >> 18)));
      s.push_back(static_cast<char>(0x80 | ((cp >> 12) & 0x3F)));
      s.push_back(static_cast<char>(0x80 | ((cp >> 6) & 0x3F)));
      s.push_back(static_cast<char>(0x80 | (cp & 0x3F)));
    }
  }
  bool hex4(unsigned& out) {
    if (i_ + 4 > t_.size()) return false;
    out = 0;
    for (int k = 0; k < 4; ++k) {
      char c = t_[i_++];
      out <<= 4;
      if (c >= '0' && c <= '9') out |= static_cast<unsigned>(c - '0');
      else if ((c | 0x20) >= 'a' && (c | 0x20) <= 'f') out |= static_cast<unsigned>((c | 0x20) - 'a' + 10);
      else return false;
    }
    return true;
  }
  bool string(std::string& s) {
    ++i_;
    while (i_ < t_.size()) {
      char c = t_[i_++];
      if (c == '"') return true;
      if (c != '\\') {
        s.push_back(c);
        continue;
      }
      if (i_ >= t_.size()) return false;
      char e = t_[i_++];
      switch (e) {
        case '"': s.push_back('"'); break;
        case '\\': s.push_back('\\'); break;
        case '/': s.push_back('/'); break;
        case 'b': s.push_back('\b'); break;
        case 'f': s.push_back('\f'); break;
        case 'n': s.push_back('\n'); break;
        case 'r': s.push_back('\r'); break;
        case 't': s.push_back('\t'); break;
        case 'u': {
          unsigned cp;
          if (!hex4(cp)) return false;
          if (cp >= 0xD800 && cp < 0xDC00 && lit("\\u")) {
            unsigned lo;
            if (!hex4(lo)) return false;
            cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
          }
          utf8(s, cp);
          break;
        }
        default: return false;
      }
    }
    return false;
  }

  std::string_view t_;
  std::size_t i_ = 0;
};

/// A JSON document as a value, in `alloc`.
inline guatiao::result<guatiao::value> read_json(std::string_view text,
                                                 const guatiao_alloc* alloc = guatiao::system_alloc()) {
  json_reader r(text);
  guatiao::item it;
  if (!r.read(it)) return guatiao::error(guatiao::status::bad_value, "not JSON at byte " + std::to_string(r.at()));
  return guatiao::try_make(it, alloc);
}

}  // namespace gt_test

#endif  // GT_TEST_JSON_HPP
