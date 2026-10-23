// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// `guatiao_intake.h`'s constant, written out so the web needs no generated
// bindings; `test/intake_test.dart` compares it on the VM.

/// The key of a provider's `meta` holding its form document: read it with
/// `Registry.providerMeta` (package:guatiao), then `check` it against
/// `providerConfig`.
const String providerFormKey = 'form';
