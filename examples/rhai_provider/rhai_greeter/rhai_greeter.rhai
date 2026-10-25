// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Greeters written in Rhai: the same kinds the Rust, C, Python and Lua
// greeters serve, offered to any host through the guatiao-source-rhai source.
//
// The script is the library: its id is its file's name, its version the
// `VERSION` below, its providers the maps its last expression names. A
// provider is a map whose function members are the kinds' slots; `this`
// in one is the map itself, which is where its state lives.
// `rhai_greeter.guatiao.toml` beside it is what the source reads
// before running anything.

const VERSION = "1.0.0";

// Changed by the examples to show a refresh serving new code.
const GREETING = "hello";

// A refusal is a `throw`: its text is the words the caller gets.
let greeting = |name| {
    if name == "" {
        throw "nobody to greet";
    }
    #{ greeting: `${GREETING}, ${name}` }
};

// One conversation `start` hands back: the listener it was handed, and
// what was said. It is an object of the `conversation` kind, so its
// function members are that kind's slots.
let new_conversation = |listener| #{
    listener: listener,
    said: [],
    say: |what| {
        if what == "" {
            throw "nothing to say";
        }
        this.said.push(what);
        this.listener.heard(what);
    },
    // `dst` is the caller's buffer; the slot answers how much it wrote.
    read: |dst| {
        let text = "";
        for said in this.said {
            if text != "" {
                text += " ";
            }
            text += said;
        }
        let n = if text.len() < dst.len() { text.len() } else { dst.len() };
        dst.write_utf8(0, n, text);
        n
    },
    turns: || this.said.len(),
};

// One provider serving two kinds, keeping its count in itself.
let hello = #{
    name: "Hello, in Rhai",
    kinds: ["greeter", "counter"],
    asked: 0,

    greet: |name| {
        if name == "panic" {
            // Anything but a `throw` is a bug in the library.
            let nobody = name.len() - name.len();
            return 1 / nobody;
        }
        greeting.call(name)
    },

    // `config` is lent: what the script changes in it is the caller's.
    check: |config| {
        if config.name == () {
            config.name = "ana";
        }
        config.checked_by = "Rhai";
        true
    },

    greet_from: |config| greeting.call(if config.name == () { "" } else { config.name }),

    greet_into: |name, listener| {
        let answer = greeting.call(name);
        listener.heard(answer.greeting);
    },

    start: |listener| new_conversation.call(listener),

    count: || {
        this.asked += 1;
        this.asked
    },
};

// A refusal with a status of its own. `shout` is left to the kind's
// default.
let polite = #{
    kinds: ["greeter"],
    greet: |name| {
        if name == "" {
            throw guatiao::refuse("bad_value", "nobody to greet, politely");
        }
        #{ greeting: `good day, ${name}` }
    },
};

#{ providers: #{ hello: hello, polite: polite } }
