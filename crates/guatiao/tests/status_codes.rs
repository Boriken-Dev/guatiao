// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A `guatiao_status` is a `uint32_t`, and a library may return a code of
//! its own: the value it reads back is a status this build does not name,
//! never an invalid one.

use guatiao::Status;

/// A C library following the header: 0 is success, anything else is
/// failure, and 1 is a code of its own.
extern "C" fn c_slot_returning_one() -> u32 {
    1
}

#[test]
fn a_code_c_returned_through_a_slot_is_a_status() {
    // SAFETY: `Status` is `repr(transparent)` over `u32`, so the two
    // function pointer types have the same ABI.
    let slot: extern "C" fn() -> Status = unsafe {
        core::mem::transmute::<extern "C" fn() -> u32, extern "C" fn() -> Status>(
            c_slot_returning_one,
        )
    };
    let status = slot();
    assert!(!status.is_ok());
    assert_ne!(status, Status::GUATIAO_OK);
    assert_eq!(u32::from(status), 1);
    assert_eq!(status.to_string(), "status 1");
    assert_eq!(format!("{status:?}"), "Status(1)");
}

#[test]
fn every_bit_pattern_reads_as_a_status() {
    for bits in [0u32, 1, 6, 15, 99, 101, u32::MAX] {
        // SAFETY: every `u32` is a valid `Status`; this is what reading a
        // field a foreign side wrote amounts to.
        let status: Status = unsafe { core::mem::transmute::<u32, Status>(bits) };
        assert_eq!(status, Status::from(bits));
        assert_eq!(status.is_ok(), bits == 0);
    }
}

#[test]
fn the_named_codes_keep_their_numbers() {
    for (status, code, name) in [
        (Status::GUATIAO_OK, 0, "GUATIAO_OK"),
        (Status::GUATIAO_ERR_BAD_VALUE, 7, "GUATIAO_ERR_BAD_VALUE"),
        (Status::GUATIAO_ERR_ALLOC, 8, "GUATIAO_ERR_ALLOC"),
        (Status::GUATIAO_ERR_WRONG_KIND, 9, "GUATIAO_ERR_WRONG_KIND"),
        (Status::GUATIAO_ERR_NOT_FOUND, 10, "GUATIAO_ERR_NOT_FOUND"),
        (Status::GUATIAO_ERR_NULL, 11, "GUATIAO_ERR_NULL"),
        (Status::GUATIAO_ERR_GONE, 12, "GUATIAO_ERR_GONE"),
        (
            Status::GUATIAO_ERR_UNSUPPORTED,
            13,
            "GUATIAO_ERR_UNSUPPORTED",
        ),
        (Status::GUATIAO_ERR_BUSY, 14, "GUATIAO_ERR_BUSY"),
        (Status::GUATIAO_ERR_INTERNAL, 100, "GUATIAO_ERR_INTERNAL"),
    ] {
        assert_eq!(u32::from(status), code);
        assert_eq!(format!("{status:?}"), name);
        assert!(!status.to_string().contains("status "), "{status}");
    }
}
