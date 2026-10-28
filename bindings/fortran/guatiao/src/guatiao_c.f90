! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> guatiao.h in Fortran: the structs as bind(c) derived types, the tag
!> and status constants, and an abstract interface and a procedure
!> pointer for every export this binding calls, resolved at run time.
!>
!> The payload union is mirrored by its 32-byte arm (`gt_owned`): string,
!> number, bytes, list and map share that layout, and a boolean is the
!> first byte of `ptr`. Every size below is the header's on a 64-bit
!> platform; the tests pin them.
module guatiao_c
  use, intrinsic :: iso_c_binding
  use guatiao_loader, only: gt_dl_sym
  implicit none
  public

  !> The kinds of a stored value (`enum guatiao_tag`).
  integer(c_int32_t), parameter :: GUATIAO_TAG_ABSENT = 0, GUATIAO_TAG_NULL = 1, GUATIAO_TAG_BOOL = 2, &
                                   GUATIAO_TAG_NUMBER = 3, GUATIAO_TAG_STRING = 4, GUATIAO_TAG_BYTES = 5, &
                                   GUATIAO_TAG_LIST = 6, GUATIAO_TAG_MAP = 7

  !> The codes of a `guatiao_status` (`enum guatiao_status_code`).
  integer(c_int32_t), parameter :: GUATIAO_OK = 0, GUATIAO_ERR_BAD_VALUE = 7, GUATIAO_ERR_ALLOC = 8, &
                                   GUATIAO_ERR_WRONG_KIND = 9, GUATIAO_ERR_NOT_FOUND = 10, &
                                   GUATIAO_ERR_NULL = 11, GUATIAO_ERR_GONE = 12, &
                                   GUATIAO_ERR_UNSUPPORTED = 13, GUATIAO_ERR_BUSY = 14, &
                                   GUATIAO_ERR_INTERNAL = 100

  integer(c_int32_t), parameter :: GUATIAO_ABI_VERSION = 1

  !> C's `bool` crosses as `integer(c_int8_t)`, 0 or 1: Intel ifx stores a
  !> `logical(c_bool)` .true. as -1 unless built with `-fpscomp logicals`,
  !> which no C or Rust reader accepts.
  integer(c_int8_t), parameter :: GUATIAO_C_TRUE = 1_c_int8_t, GUATIAO_C_FALSE = 0_c_int8_t
  integer, parameter :: GUATIAO_MAX_DEPTH = 128

  !> `guatiao_str` and `guatiao_bytes`: a view, pointer and length.
  type, bind(c) :: gt_str
    type(c_ptr) :: ptr = c_null_ptr
    integer(c_size_t) :: len = 0
  end type

  !> `guatiao_string`, `guatiao_buffer`, `guatiao_list`, `guatiao_map`:
  !> an owned array; `cap == 0` owns nothing.
  type, bind(c) :: gt_owned
    type(c_ptr) :: ptr = c_null_ptr
    integer(c_size_t) :: len = 0
    integer(c_size_t) :: cap = 0
    type(c_ptr) :: alloc = c_null_ptr
  end type

  !> `guatiao_value`: 8 bytes and four words.
  type, bind(c) :: gt_value
    integer(c_int32_t) :: tag = GUATIAO_TAG_ABSENT
    integer(c_int32_t) :: pad = 0
    type(gt_owned) :: payload
  end type

  !> `guatiao_entry`: 8 bytes and eight words.
  type, bind(c) :: gt_entry
    type(gt_owned) :: key
    type(gt_value) :: value
  end type

  !> `guatiao_alloc`: five words.
  type, bind(c) :: gt_alloc
    integer(c_int32_t) :: struct_size = 0
    type(c_ptr) :: ctx = c_null_ptr
    type(c_funptr) :: alloc = c_null_funptr
    type(c_funptr) :: free = c_null_funptr
    type(c_funptr) :: release = c_null_funptr
  end type

  !> `guatiao_provider_error`: five words.
  type, bind(c) :: gt_provider_error
    integer(c_int32_t) :: status = GUATIAO_OK
    type(gt_owned) :: message
  end type

  !> `guatiao_kinds`: two words.
  type, bind(c) :: gt_kinds
    type(c_ptr) :: ptr = c_null_ptr
    integer(c_size_t) :: len = 0
  end type

  !> `guatiao_kind_table`: 8 bytes and three words.
  type, bind(c) :: gt_kind_table
    integer(c_int32_t) :: struct_size = 0
    integer(c_int32_t) :: vtable_size = 0
    type(gt_str) :: kind
    type(c_ptr) :: vtable = c_null_ptr
  end type

  !> `guatiao_kind_tables`: three words.
  type, bind(c) :: gt_kind_tables
    type(c_ptr) :: ptr = c_null_ptr
    integer(c_size_t) :: len = 0
    integer(c_size_t) :: stride = 0
  end type

  !> `guatiao_provider_info`: 8 bytes and 20 words.
  type, bind(c) :: gt_provider_info
    integer(c_int32_t) :: struct_size = 0
    integer(c_int32_t) :: vtable_size = 0
    type(gt_kinds) :: kinds
    type(gt_str) :: id
    type(gt_str) :: display_name
    type(c_ptr) :: config = c_null_ptr
    type(c_ptr) :: vtable = c_null_ptr
    type(c_ptr) :: ctx = c_null_ptr
    type(c_ptr) :: meta = c_null_ptr
    type(gt_str) :: version
    type(c_funptr) :: available = c_null_funptr
    type(gt_kind_tables) :: tables
    type(c_funptr) :: create = c_null_funptr
    type(c_funptr) :: destroy = c_null_funptr
    type(gt_str) :: kind_descriptions
  end type

  !> `guatiao_host_services`: nine words.
  type, bind(c) :: gt_host_services
    integer(c_int32_t) :: struct_size = 0
    type(c_ptr) :: ctx = c_null_ptr
    type(c_funptr) :: get = c_null_funptr
    type(c_funptr) :: list = c_null_funptr
    type(c_funptr) :: alloc = c_null_funptr
    type(c_funptr) :: get_pinned = c_null_funptr
    type(c_funptr) :: list_pinned = c_null_funptr
    type(c_funptr) :: pin = c_null_funptr
    type(c_funptr) :: unpin = c_null_funptr
  end type

  !> `guatiao_host_info`: 8 bytes and seven words.
  type, bind(c) :: gt_host_info
    integer(c_int32_t) :: struct_size = 0
    integer(c_int32_t) :: abi_version = 0
    type(gt_str) :: host_id
    type(gt_str) :: host_version
    type(c_ptr) :: alloc = c_null_ptr
    type(c_ptr) :: meta = c_null_ptr
    type(c_ptr) :: services = c_null_ptr
  end type

  !> `guatiao_providers`: three words.
  type, bind(c) :: gt_providers
    type(c_ptr) :: ptr = c_null_ptr
    integer(c_size_t) :: len = 0
    integer(c_size_t) :: stride = 0
  end type

  !> `guatiao_library_info`: 8 bytes and nine words.
  type, bind(c) :: gt_library_info
    integer(c_int32_t) :: struct_size = 0
    integer(c_int32_t) :: abi_version = 0
    type(gt_str) :: id
    type(gt_str) :: version
    type(gt_providers) :: providers
    type(c_ptr) :: meta = c_null_ptr
    type(c_funptr) :: unload = c_null_funptr
  end type

  !> `guatiao_span`: 64 bytes, one sign for every field.
  type, bind(c) :: gt_span
    integer(c_int64_t) :: years = 0, months = 0, weeks = 0, days = 0
    integer(c_int64_t) :: hours = 0, minutes = 0, seconds = 0
    integer(c_int32_t) :: nanoseconds = 0
  end type

  !> `guatiao_text_error`: 8 bytes and five words.
  type, bind(c) :: gt_text_error
    integer(c_size_t) :: at = 0
    type(gt_value) :: expected
  end type

  !> `guatiao_date`: 8 bytes; `month` and `day` are `uint8_t`.
  type, bind(c) :: gt_date
    integer(c_int32_t) :: year = 0
    integer(c_int8_t) :: month = 0
    integer(c_int8_t) :: day = 0
  end type

  !> `guatiao_time`: 8 bytes; the first three are `uint8_t`.
  type, bind(c) :: gt_time
    integer(c_int8_t) :: hour = 0
    integer(c_int8_t) :: minute = 0
    integer(c_int8_t) :: second = 0
    integer(c_int32_t) :: nanosecond = 0
  end type

  !> `guatiao_datetime`: 16 bytes.
  type, bind(c) :: gt_datetime
    type(gt_date) :: date
    type(gt_time) :: time
  end type

  !> `guatiao_timestamp`: 16 bytes; `nanoseconds` is `uint32_t`.
  type, bind(c) :: gt_timestamp
    integer(c_int64_t) :: seconds = 0
    integer(c_int32_t) :: nanoseconds = 0
  end type

  !> `guatiao_kind_header`: the first 8 bytes of every kind table.
  type, bind(c) :: gt_kind_header
    integer(c_int32_t) :: struct_size = 0
    integer(c_int32_t) :: floor_hash = 0
  end type

  !> `guatiao_object`: three words; ownership crosses with it.
  type, bind(c) :: gt_object
    type(c_ptr) :: table = c_null_ptr
    integer(c_size_t) :: size = 0
    type(c_ptr) :: ctx = c_null_ptr
  end type

  !> `guatiao_bytes_mut`: 16 bytes, writable.
  type, bind(c) :: gt_bytes_mut
    type(c_ptr) :: ptr = c_null_ptr
    integer(c_size_t) :: len = 0
  end type

  !> `guatiao_wire_event`: 144 bytes. What a channel frame said, or why
  !> it was refused; its three values are the caller's.
  type, bind(c) :: gt_wire_event
    integer(c_int64_t) :: channel = 0
    integer(c_int32_t) :: kind = 0
    integer(c_int32_t) :: refusal = 0
    integer(c_size_t) :: at = 0
    type(gt_value) :: value
    type(gt_value) :: message
    type(gt_value) :: detail
  end type

  !> `GUATIAO_LEN_UNKNOWN`, UINT64_MAX: a container that did not say how
  !> many items follow.
  integer(c_int64_t), parameter :: GUATIAO_LEN_UNKNOWN = -1_c_int64_t

  !> `GUATIAO_IMPORT_SEARCH_*`: where Windows finds what a file a registry
  !> opens imports (`guatiao_registry_set_import_search`).
  integer(c_int32_t), parameter :: GUATIAO_IMPORT_SEARCH_LIBRARY = 0, GUATIAO_IMPORT_SEARCH_PATH = 1

  !> `guatiao_visitor`: thirteen words. What receives a walk's events:
  !> `struct_size` (`c_sizeof` of this type), `ctx` handed back to each
  !> function, then a function per event; a null one refuses its event
  !> with GUATIAO_ERR_WRONG_KIND.
  type, bind(c) :: gt_visitor
    integer(c_int32_t) :: struct_size = 0
    type(c_ptr) :: ctx = c_null_ptr
    type(c_funptr) :: absent = c_null_funptr
    type(c_funptr) :: null = c_null_funptr
    type(c_funptr) :: boolean = c_null_funptr
    type(c_funptr) :: number = c_null_funptr
    type(c_funptr) :: string = c_null_funptr
    type(c_funptr) :: bytes = c_null_funptr
    type(c_funptr) :: list_begin = c_null_funptr
    type(c_funptr) :: list_end = c_null_funptr
    type(c_funptr) :: map_begin = c_null_funptr
    type(c_funptr) :: key = c_null_funptr
    type(c_funptr) :: map_end = c_null_funptr
  end type

  !> `guatiao_walker`: three words. What hands its own data to a visitor:
  !> `walk(ctx, into)` writes one value's events into `into`.
  type, bind(c) :: gt_walker
    integer(c_int32_t) :: struct_size = 0
    type(c_ptr) :: ctx = c_null_ptr
    type(c_funptr) :: walk = c_null_funptr
  end type

  !> One export this binding calls: its name, how many arguments the
  !> header gives it, and the build feature it needs ('' for none).
  type :: gt_export
    character(len=40) :: name
    integer :: nargs
    character(len=8) :: feature
  end type

  type(gt_export), parameter :: gt_core_exports(86) = [ &
    gt_export('guatiao_alloc_default              ', 1, ''), &
    gt_export('guatiao_value_free                 ', 1, ''), &
    gt_export('guatiao_value_clone                ', 3, ''), &
    gt_export('guatiao_value_make_local           ', 2, ''), &
    gt_export('guatiao_map_settle_lent            ', 2, ''), &
    gt_export('guatiao_kind_document_read         ', 4, ''), &
    gt_export('guatiao_value_null                 ', 1, ''), &
    gt_export('guatiao_value_absent               ', 1, ''), &
    gt_export('guatiao_value_bool                 ', 2, ''), &
    gt_export('guatiao_value_map                  ', 2, ''), &
    gt_export('guatiao_value_list                 ', 2, ''), &
    gt_export('guatiao_value_string               ', 3, ''), &
    gt_export('guatiao_value_number               ', 3, ''), &
    gt_export('guatiao_value_bytes                ', 3, ''), &
    gt_export('guatiao_map_set                    ', 4, ''), &
    gt_export('guatiao_map_discard                ', 2, ''), &
    gt_export('guatiao_map_clear                  ', 1, ''), &
    gt_export('guatiao_map_copy_from              ', 3, ''), &
    gt_export('guatiao_list_push                  ', 3, ''), &
    gt_export('guatiao_list_discard               ', 2, ''), &
    gt_export('guatiao_list_clear                 ', 1, ''), &
    gt_export('guatiao_string_push                ', 3, ''), &
    gt_export('guatiao_buffer_push                ', 3, ''), &
    gt_export('guatiao_schema_validate            ', 4, ''), &
    gt_export('guatiao_schema_redact              ', 5, ''), &
    gt_export('guatiao_date_parse                 ', 4, ''), &
    gt_export('guatiao_date_format                ', 3, ''), &
    gt_export('guatiao_date_normalize             ', 4, ''), &
    gt_export('guatiao_date_add_span              ', 3, ''), &
    gt_export('guatiao_time_parse                 ', 4, ''), &
    gt_export('guatiao_time_format                ', 3, ''), &
    gt_export('guatiao_time_normalize             ', 4, ''), &
    gt_export('guatiao_datetime_parse             ', 4, ''), &
    gt_export('guatiao_datetime_format            ', 3, ''), &
    gt_export('guatiao_datetime_normalize         ', 4, ''), &
    gt_export('guatiao_datetime_add_span          ', 3, ''), &
    gt_export('guatiao_timestamp_parse            ', 4, ''), &
    gt_export('guatiao_timestamp_format           ', 3, ''), &
    gt_export('guatiao_timestamp_normalize        ', 4, ''), &
    gt_export('guatiao_span_parse                 ', 4, ''), &
    gt_export('guatiao_span_format                ', 3, ''), &
    gt_export('guatiao_span_normalize             ', 4, ''), &
    gt_export('guatiao_duration_read              ', 3, ''), &
    gt_export('guatiao_wire_encode                ', 3, ''), &
    gt_export('guatiao_wire_decode                ', 3, ''), &
    gt_export('guatiao_wire_announce              ', 4, ''), &
    gt_export('guatiao_wire_send                  ', 4, ''), &
    gt_export('guatiao_wire_close                 ', 3, ''), &
    gt_export('guatiao_wire_receiver_new          ', 1, ''), &
    gt_export('guatiao_wire_receiver_accept       ', 3, ''), &
    gt_export('guatiao_wire_receiver_free         ', 1, ''), &
    gt_export('guatiao_registry_new               ', 3, 'load'), &
    gt_export('guatiao_registry_free              ', 1, 'load'), &
    gt_export('guatiao_registry_host              ', 1, 'load'), &
    gt_export('guatiao_registry_keyed_by          ', 2, 'load'), &
    gt_export('guatiao_registry_libraries_keyed_by', 2, 'load'), &
    gt_export('guatiao_registry_load_file         ', 4, 'load'), &
    gt_export('guatiao_registry_register_entry    ', 5, 'load'), &
    gt_export('guatiao_registry_add_source_file   ', 4, 'load'), &
    gt_export('guatiao_registry_watch_dir         ', 3, 'load'), &
    gt_export('guatiao_registry_refresh           ', 3, 'load'), &
    gt_export('guatiao_registry_scan_dir          ', 5, 'load'), &
    gt_export('guatiao_registry_scan_dir_rules    ', 6, 'load'), &
    gt_export('guatiao_registry_scan_path         ', 6, 'load'), &
    gt_export('guatiao_registry_libraries         ', 3, 'load'), &
    gt_export('guatiao_registry_providers         ', 4, 'load'), &
    gt_export('guatiao_registry_provider          ', 4, 'load'), &
    gt_export('guatiao_registry_available         ', 4, 'load'), &
    gt_export('guatiao_registry_why_not           ', 4, 'load'), &
    gt_export('guatiao_registry_best              ', 4, 'load'), &
    gt_export('guatiao_registry_provider_available', 3, 'load'), &
    gt_export('guatiao_registry_set_priority      ', 3, 'load'), &
    gt_export('guatiao_registry_priority          ', 2, 'load'), &
    gt_export('guatiao_registry_set_import_search ', 2, 'load'), &
    gt_export('guatiao_registry_import_search     ', 1, 'load'), &
    gt_export('guatiao_registry_provider_vtable   ', 3, 'load'), &
    gt_export('guatiao_registry_provider_table    ', 4, 'load'), &
    gt_export('guatiao_registry_provider_ctx      ', 2, 'load'), &
    gt_export('guatiao_registry_provider_config   ', 2, 'load'), &
    gt_export('guatiao_registry_provider_meta     ', 4, 'load'), &
    gt_export('guatiao_registry_provider_create   ', 5, 'load'), &
    gt_export('guatiao_registry_provider_destroy  ', 3, 'load'), &
    gt_export('guatiao_registry_retire            ', 2, 'load'), &
    gt_export('guatiao_registry_unload            ', 2, 'load'), &
    gt_export('guatiao_registry_unload_unchecked  ', 2, 'load'), &
    gt_export('guatiao_registry_collect           ', 3, 'load') ]

  abstract interface
    function guatiao_alloc_default_i(out) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_value_free_i(v) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: v
      integer(c_int32_t) :: r
    end function
    function guatiao_value_clone_i(alloc, src, out) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: alloc
      type(c_ptr), value :: src
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_value_make_local_i(v, alloc) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: v
      type(c_ptr), value :: alloc
      integer(c_int32_t) :: r
    end function
    function guatiao_map_settle_lent_i(node, home) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: node
      type(c_ptr), value :: home
      integer(c_int32_t) :: r
    end function
    function guatiao_kind_document_read_i(text, alloc, out, why) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(gt_str), value :: text
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out
      type(c_ptr), value :: why
      integer(c_int32_t) :: r
    end function
    function guatiao_value_null_i(out) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_value_absent_i(out) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_value_bool_i(b, out) bind(c) result(r)
      import :: c_int8_t, c_int32_t, c_ptr
      integer(c_int8_t), value :: b
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_value_map_i(alloc, out) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_value_list_i(alloc, out) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_value_string_i(alloc, text, out) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: alloc
      type(gt_str), value :: text
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_value_number_i(alloc, text, out) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: alloc
      type(gt_str), value :: text
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_value_bytes_i(alloc, bytes, out) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: alloc
      type(gt_str), value :: bytes
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_map_set_i(alloc, node, key, value) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: alloc
      type(c_ptr), value :: node
      type(gt_str), value :: key
      type(c_ptr), value :: value
      integer(c_int32_t) :: r
    end function
    function guatiao_map_discard_i(node, key) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: node
      type(gt_str), value :: key
      integer(c_int32_t) :: r
    end function
    function guatiao_map_clear_i(node) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: node
      integer(c_int32_t) :: r
    end function
    function guatiao_map_copy_from_i(alloc, dst, src) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: alloc
      type(c_ptr), value :: dst
      type(c_ptr), value :: src
      integer(c_int32_t) :: r
    end function
    function guatiao_list_push_i(alloc, node, value) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: alloc
      type(c_ptr), value :: node
      type(c_ptr), value :: value
      integer(c_int32_t) :: r
    end function
    function guatiao_list_discard_i(node, index) bind(c) result(r)
      import :: c_int32_t, c_ptr, c_size_t
      type(c_ptr), value :: node
      integer(c_size_t), value :: index
      integer(c_int32_t) :: r
    end function
    function guatiao_list_clear_i(node) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: node
      integer(c_int32_t) :: r
    end function
    function guatiao_string_push_i(alloc, node, text) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: alloc
      type(c_ptr), value :: node
      type(gt_str), value :: text
      integer(c_int32_t) :: r
    end function
    function guatiao_buffer_push_i(alloc, node, bytes) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: alloc
      type(c_ptr), value :: node
      type(gt_str), value :: bytes
      integer(c_int32_t) :: r
    end function
    function guatiao_schema_validate_i(schema, config, alloc, out_error) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: schema
      type(c_ptr), value :: config
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out_error
      integer(c_int32_t) :: r
    end function
    function guatiao_schema_redact_i(schema, value, placeholder, alloc, out) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: schema
      type(c_ptr), value :: value
      type(gt_str), value :: placeholder
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_date_parse_i(alloc, text, out, out_error) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: alloc
      type(gt_str), value :: text
      type(c_ptr), value :: out
      type(c_ptr), value :: out_error
      integer(c_int32_t) :: r
    end function
    function guatiao_date_format_i(alloc, date, out) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: alloc
      type(c_ptr), value :: date
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_date_normalize_i(alloc, text, out, out_error) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: alloc
      type(gt_str), value :: text
      type(c_ptr), value :: out
      type(c_ptr), value :: out_error
      integer(c_int32_t) :: r
    end function
    function guatiao_date_add_span_i(date, span, out) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: date
      type(c_ptr), value :: span
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_time_parse_i(alloc, text, out, out_error) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: alloc
      type(gt_str), value :: text
      type(c_ptr), value :: out
      type(c_ptr), value :: out_error
      integer(c_int32_t) :: r
    end function
    function guatiao_time_format_i(alloc, time, out) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: alloc
      type(c_ptr), value :: time
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_time_normalize_i(alloc, text, out, out_error) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: alloc
      type(gt_str), value :: text
      type(c_ptr), value :: out
      type(c_ptr), value :: out_error
      integer(c_int32_t) :: r
    end function
    function guatiao_datetime_parse_i(alloc, text, out, out_error) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: alloc
      type(gt_str), value :: text
      type(c_ptr), value :: out
      type(c_ptr), value :: out_error
      integer(c_int32_t) :: r
    end function
    function guatiao_datetime_format_i(alloc, datetime, out) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: alloc
      type(c_ptr), value :: datetime
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_datetime_normalize_i(alloc, text, out, out_error) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: alloc
      type(gt_str), value :: text
      type(c_ptr), value :: out
      type(c_ptr), value :: out_error
      integer(c_int32_t) :: r
    end function
    function guatiao_datetime_add_span_i(datetime, span, out) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: datetime
      type(c_ptr), value :: span
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_timestamp_parse_i(alloc, text, out, out_error) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: alloc
      type(gt_str), value :: text
      type(c_ptr), value :: out
      type(c_ptr), value :: out_error
      integer(c_int32_t) :: r
    end function
    function guatiao_timestamp_format_i(alloc, timestamp, out) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: alloc
      type(c_ptr), value :: timestamp
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_timestamp_normalize_i(alloc, text, out, out_error) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: alloc
      type(gt_str), value :: text
      type(c_ptr), value :: out
      type(c_ptr), value :: out_error
      integer(c_int32_t) :: r
    end function
    function guatiao_span_parse_i(alloc, text, out, out_error) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: alloc
      type(gt_str), value :: text
      type(c_ptr), value :: out
      type(c_ptr), value :: out_error
      integer(c_int32_t) :: r
    end function
    function guatiao_span_format_i(alloc, span, out) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: alloc
      type(c_ptr), value :: span
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_span_normalize_i(alloc, text, out, out_error) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: alloc
      type(gt_str), value :: text
      type(c_ptr), value :: out
      type(c_ptr), value :: out_error
      integer(c_int32_t) :: r
    end function
    function guatiao_duration_read_i(value, out_seconds, out_nanoseconds) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: value
      type(c_ptr), value :: out_seconds
      type(c_ptr), value :: out_nanoseconds
      integer(c_int32_t) :: r
    end function
    function guatiao_registry_new_i(id, version, alloc) bind(c) result(r)
      import :: c_ptr, gt_str
      type(gt_str), value :: id
      type(gt_str), value :: version
      type(c_ptr), value :: alloc
      type(c_ptr) :: r
    end function
    subroutine guatiao_registry_free_i(reg) bind(c)
      import :: c_ptr
      type(c_ptr), value :: reg
    end subroutine
    function guatiao_registry_host_i(reg) bind(c) result(r)
      import :: c_ptr
      type(c_ptr), value :: reg
      type(c_ptr) :: r
    end function
    function guatiao_registry_keyed_by_i(reg, template_) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: reg
      type(gt_str), value :: template_
      integer(c_int32_t) :: r
    end function
    function guatiao_registry_libraries_keyed_by_i(reg, template_) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: reg
      type(gt_str), value :: template_
      integer(c_int32_t) :: r
    end function
    function guatiao_registry_load_file_i(reg, path, alloc, out) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: reg
      type(gt_str), value :: path
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_registry_register_entry_i(reg, name, entry, alloc, out) bind(c) result(r)
      import :: c_funptr, c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: reg
      type(gt_str), value :: name
      type(c_funptr), value :: entry
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_registry_add_source_file_i(reg, path, config, err) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: reg
      type(gt_str), value :: path
      type(c_ptr), value :: config
      type(c_ptr), value :: err
      integer(c_int32_t) :: r
    end function
    function guatiao_registry_watch_dir_i(reg, dir, rules) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: reg
      type(gt_str), value :: dir
      type(gt_str), value :: rules
      integer(c_int32_t) :: r
    end function
    function guatiao_registry_refresh_i(reg, alloc, out) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: reg
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_registry_scan_dir_i(reg, dir, descending, alloc, out) bind(c) result(r)
      import :: c_int8_t, c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: reg
      type(gt_str), value :: dir
      integer(c_int8_t), value :: descending
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_registry_scan_dir_rules_i(reg, dir, descending, rules, alloc, out) bind(c) result(r)
      import :: c_int8_t, c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: reg
      type(gt_str), value :: dir
      integer(c_int8_t), value :: descending
      type(gt_str), value :: rules
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_registry_scan_path_i(reg, spec, descending, rules, alloc, out) bind(c) result(r)
      import :: c_int8_t, c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: reg
      type(gt_str), value :: spec
      integer(c_int8_t), value :: descending
      type(gt_str), value :: rules
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    ! scan_dir_rules and scan_path with `rules` passed as its two words,
    ! which is the same call on every 64-bit ABI but Windows' (where the
    ! struct goes by address). LLVM flang 22 on x86-64, calling through a
    ! pointer, passes a struct that lands in the last two integer registers
    ! on the stack instead.
    function gt_scan_split(reg, where, descending, rules_ptr, rules_len, alloc, out) bind(c) result(r)
      import :: c_int8_t, c_int32_t, c_ptr, c_size_t, gt_str
      type(c_ptr), value :: reg
      type(gt_str), value :: where
      integer(c_int8_t), value :: descending
      type(c_ptr), value :: rules_ptr
      integer(c_size_t), value :: rules_len
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_registry_libraries_i(reg, alloc, out) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: reg
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_registry_providers_i(reg, kind, alloc, out) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: reg
      type(gt_str), value :: kind
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_registry_provider_i(reg, key, alloc, out) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: reg
      type(gt_str), value :: key
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_registry_available_i(reg, kind, alloc, out) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: reg
      type(gt_str), value :: kind
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_registry_why_not_i(reg, kind, alloc, out) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: reg
      type(gt_str), value :: kind
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_registry_best_i(reg, kind, alloc, out) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: reg
      type(gt_str), value :: kind
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_registry_provider_available_i(reg, key, reason) bind(c) result(r)
      import :: c_int8_t, c_ptr, gt_str
      type(c_ptr), value :: reg
      type(gt_str), value :: key
      type(c_ptr), value :: reason
      integer(c_int8_t) :: r
    end function
    function guatiao_registry_set_priority_i(reg, id, priority) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: reg
      type(gt_str), value :: id
      integer(c_int32_t), value :: priority
      integer(c_int32_t) :: r
    end function
    function guatiao_registry_priority_i(reg, id) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: reg
      type(gt_str), value :: id
      integer(c_int32_t) :: r
    end function
    function guatiao_registry_set_import_search_i(reg, search) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: reg
      integer(c_int32_t), value :: search
      integer(c_int32_t) :: r
    end function
    function guatiao_registry_import_search_i(reg) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: reg
      integer(c_int32_t) :: r
    end function
    function guatiao_registry_provider_vtable_i(reg, key, size_out) bind(c) result(r)
      import :: c_ptr, gt_str
      type(c_ptr), value :: reg
      type(gt_str), value :: key
      type(c_ptr), value :: size_out
      type(c_ptr) :: r
    end function
    function guatiao_registry_provider_table_i(reg, key, kind, size_out) bind(c) result(r)
      import :: c_ptr, gt_str
      type(c_ptr), value :: reg
      type(gt_str), value :: key
      type(gt_str), value :: kind
      type(c_ptr), value :: size_out
      type(c_ptr) :: r
    end function
    function guatiao_registry_provider_ctx_i(reg, key) bind(c) result(r)
      import :: c_ptr, gt_str
      type(c_ptr), value :: reg
      type(gt_str), value :: key
      type(c_ptr) :: r
    end function
    function guatiao_registry_provider_config_i(reg, key) bind(c) result(r)
      import :: c_ptr, gt_str
      type(c_ptr), value :: reg
      type(gt_str), value :: key
      type(c_ptr) :: r
    end function
    function guatiao_registry_provider_meta_i(reg, key, alloc, out) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: reg
      type(gt_str), value :: key
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_registry_provider_create_i(reg, key, config, out, err) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: reg
      type(gt_str), value :: key
      type(c_ptr), value :: config
      type(c_ptr), value :: out
      type(c_ptr), value :: err
      integer(c_int32_t) :: r
    end function
    subroutine guatiao_registry_provider_destroy_i(reg, key, instance) bind(c)
      import :: c_ptr, gt_str
      type(c_ptr), value :: reg
      type(gt_str), value :: key
      type(c_ptr), value :: instance
    end subroutine
    function guatiao_registry_retire_i(reg, key) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: reg
      type(gt_str), value :: key
      integer(c_int32_t) :: r
    end function
    function guatiao_registry_unload_i(reg, key) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: reg
      type(gt_str), value :: key
      integer(c_int32_t) :: r
    end function
    function guatiao_registry_unload_unchecked_i(reg, key) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: reg
      type(gt_str), value :: key
      integer(c_int32_t) :: r
    end function
    function guatiao_registry_collect_i(reg, alloc, out) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: reg
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_wire_encode_i(alloc, value, out) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: alloc
      type(c_ptr), value :: value
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_wire_decode_i(alloc, bytes, out) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: alloc
      type(gt_str), value :: bytes
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_wire_announce_i(alloc, channel, schema, out) bind(c) result(r)
      import :: c_int32_t, c_int64_t, c_ptr
      type(c_ptr), value :: alloc
      integer(c_int64_t), value :: channel
      type(c_ptr), value :: schema
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_wire_send_i(alloc, channel, value, out) bind(c) result(r)
      import :: c_int32_t, c_int64_t, c_ptr
      type(c_ptr), value :: alloc
      integer(c_int64_t), value :: channel
      type(c_ptr), value :: value
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_wire_close_i(alloc, channel, out) bind(c) result(r)
      import :: c_int32_t, c_int64_t, c_ptr
      type(c_ptr), value :: alloc
      integer(c_int64_t), value :: channel
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_wire_receiver_new_i(alloc) bind(c) result(r)
      import :: c_ptr
      type(c_ptr), value :: alloc
      type(c_ptr) :: r
    end function
    function guatiao_wire_receiver_accept_i(recv, frame, out_event) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: recv
      type(gt_str), value :: frame
      type(c_ptr), value :: out_event
      integer(c_int32_t) :: r
    end function
    subroutine guatiao_wire_receiver_free_i(recv) bind(c)
      import :: c_ptr
      type(c_ptr), value :: recv
    end subroutine
  end interface

  procedure(guatiao_alloc_default_i), pointer :: gt_alloc_default => null()
  procedure(guatiao_value_free_i), pointer :: gt_value_free => null()
  procedure(guatiao_value_clone_i), pointer :: gt_value_clone => null()
  procedure(guatiao_value_make_local_i), pointer :: gt_value_make_local => null()
  procedure(guatiao_map_settle_lent_i), pointer :: gt_map_settle_lent => null()
  procedure(guatiao_kind_document_read_i), pointer :: gt_kind_document_read => null()
  procedure(guatiao_value_null_i), pointer :: gt_value_null => null()
  procedure(guatiao_value_absent_i), pointer :: gt_value_absent => null()
  procedure(guatiao_value_bool_i), pointer :: gt_value_bool => null()
  procedure(guatiao_value_map_i), pointer :: gt_value_map => null()
  procedure(guatiao_value_list_i), pointer :: gt_value_list => null()
  procedure(guatiao_value_string_i), pointer :: gt_value_string => null()
  procedure(guatiao_value_number_i), pointer :: gt_value_number => null()
  procedure(guatiao_value_bytes_i), pointer :: gt_value_bytes => null()
  procedure(guatiao_map_set_i), pointer :: gt_map_set => null()
  procedure(guatiao_map_discard_i), pointer :: gt_map_discard => null()
  procedure(guatiao_map_clear_i), pointer :: gt_map_clear => null()
  procedure(guatiao_map_copy_from_i), pointer :: gt_map_copy_from => null()
  procedure(guatiao_list_push_i), pointer :: gt_list_push => null()
  procedure(guatiao_list_discard_i), pointer :: gt_list_discard => null()
  procedure(guatiao_list_clear_i), pointer :: gt_list_clear => null()
  procedure(guatiao_string_push_i), pointer :: gt_string_push => null()
  procedure(guatiao_buffer_push_i), pointer :: gt_buffer_push => null()
  procedure(guatiao_schema_validate_i), pointer :: gt_schema_validate => null()
  procedure(guatiao_schema_redact_i), pointer :: gt_schema_redact => null()
  procedure(guatiao_date_parse_i), pointer :: gt_date_parse => null()
  procedure(guatiao_date_format_i), pointer :: gt_date_format => null()
  procedure(guatiao_date_normalize_i), pointer :: gt_date_normalize => null()
  procedure(guatiao_date_add_span_i), pointer :: gt_date_add_span => null()
  procedure(guatiao_time_parse_i), pointer :: gt_time_parse => null()
  procedure(guatiao_time_format_i), pointer :: gt_time_format => null()
  procedure(guatiao_time_normalize_i), pointer :: gt_time_normalize => null()
  procedure(guatiao_datetime_parse_i), pointer :: gt_datetime_parse => null()
  procedure(guatiao_datetime_format_i), pointer :: gt_datetime_format => null()
  procedure(guatiao_datetime_normalize_i), pointer :: gt_datetime_normalize => null()
  procedure(guatiao_datetime_add_span_i), pointer :: gt_datetime_add_span => null()
  procedure(guatiao_timestamp_parse_i), pointer :: gt_timestamp_parse => null()
  procedure(guatiao_timestamp_format_i), pointer :: gt_timestamp_format => null()
  procedure(guatiao_timestamp_normalize_i), pointer :: gt_timestamp_normalize => null()
  procedure(guatiao_span_parse_i), pointer :: gt_span_parse => null()
  procedure(guatiao_span_format_i), pointer :: gt_span_format => null()
  procedure(guatiao_span_normalize_i), pointer :: gt_span_normalize => null()
  procedure(guatiao_duration_read_i), pointer :: gt_duration_read => null()
  procedure(guatiao_registry_new_i), pointer :: gt_registry_new => null()
  procedure(guatiao_registry_free_i), pointer :: gt_registry_free => null()
  procedure(guatiao_registry_host_i), pointer :: gt_registry_host => null()
  procedure(guatiao_registry_keyed_by_i), pointer :: gt_registry_keyed_by => null()
  procedure(guatiao_registry_libraries_keyed_by_i), pointer :: gt_registry_libraries_keyed_by => null()
  procedure(guatiao_registry_load_file_i), pointer :: gt_registry_load_file => null()
  procedure(guatiao_registry_register_entry_i), pointer :: gt_registry_register_entry => null()
  procedure(guatiao_registry_add_source_file_i), pointer :: gt_registry_add_source_file => null()
  procedure(guatiao_registry_watch_dir_i), pointer :: gt_registry_watch_dir => null()
  procedure(guatiao_registry_refresh_i), pointer :: gt_registry_refresh => null()
  procedure(guatiao_registry_scan_dir_i), pointer :: gt_registry_scan_dir => null()
  procedure(guatiao_registry_scan_dir_rules_i), pointer :: gt_registry_scan_dir_rules => null()
  procedure(guatiao_registry_scan_path_i), pointer :: gt_registry_scan_path => null()
  procedure(gt_scan_split), pointer :: gt_registry_scan_dir_rules_split => null()
  procedure(gt_scan_split), pointer :: gt_registry_scan_path_split => null()
  procedure(guatiao_registry_libraries_i), pointer :: gt_registry_libraries => null()
  procedure(guatiao_registry_providers_i), pointer :: gt_registry_providers => null()
  procedure(guatiao_registry_provider_i), pointer :: gt_registry_provider => null()
  procedure(guatiao_registry_available_i), pointer :: gt_registry_available => null()
  procedure(guatiao_registry_why_not_i), pointer :: gt_registry_why_not => null()
  procedure(guatiao_registry_best_i), pointer :: gt_registry_best => null()
  procedure(guatiao_registry_provider_available_i), pointer :: gt_registry_provider_available => null()
  procedure(guatiao_registry_set_priority_i), pointer :: gt_registry_set_priority => null()
  procedure(guatiao_registry_priority_i), pointer :: gt_registry_priority => null()
  procedure(guatiao_registry_set_import_search_i), pointer :: gt_registry_set_import_search => null()
  procedure(guatiao_registry_import_search_i), pointer :: gt_registry_import_search => null()
  procedure(guatiao_registry_provider_vtable_i), pointer :: gt_registry_provider_vtable => null()
  procedure(guatiao_registry_provider_table_i), pointer :: gt_registry_provider_table => null()
  procedure(guatiao_registry_provider_ctx_i), pointer :: gt_registry_provider_ctx => null()
  procedure(guatiao_registry_provider_config_i), pointer :: gt_registry_provider_config => null()
  procedure(guatiao_registry_provider_meta_i), pointer :: gt_registry_provider_meta => null()
  procedure(guatiao_registry_provider_create_i), pointer :: gt_registry_provider_create => null()
  procedure(guatiao_registry_provider_destroy_i), pointer :: gt_registry_provider_destroy => null()
  procedure(guatiao_registry_retire_i), pointer :: gt_registry_retire => null()
  procedure(guatiao_registry_unload_i), pointer :: gt_registry_unload => null()
  procedure(guatiao_registry_unload_unchecked_i), pointer :: gt_registry_unload_unchecked => null()
  procedure(guatiao_registry_collect_i), pointer :: gt_registry_collect => null()
  procedure(guatiao_wire_encode_i), pointer :: gt_wire_encode => null()
  procedure(guatiao_wire_decode_i), pointer :: gt_wire_decode => null()
  procedure(guatiao_wire_announce_i), pointer :: gt_wire_announce => null()
  procedure(guatiao_wire_send_i), pointer :: gt_wire_send => null()
  procedure(guatiao_wire_close_i), pointer :: gt_wire_close => null()
  procedure(guatiao_wire_receiver_new_i), pointer :: gt_wire_receiver_new => null()
  procedure(guatiao_wire_receiver_accept_i), pointer :: gt_wire_receiver_accept => null()
  procedure(guatiao_wire_receiver_free_i), pointer :: gt_wire_receiver_free => null()

contains

  !> Points every export of this header at library `h`; one it lacks
  !> stays null.
  recursive subroutine gt_resolve_core(h)
    type(c_ptr), intent(in) :: h
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_alloc_default'), gt_alloc_default)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_value_free'), gt_value_free)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_value_clone'), gt_value_clone)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_value_make_local'), gt_value_make_local)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_map_settle_lent'), gt_map_settle_lent)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_kind_document_read'), gt_kind_document_read)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_value_null'), gt_value_null)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_value_absent'), gt_value_absent)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_value_bool'), gt_value_bool)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_value_map'), gt_value_map)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_value_list'), gt_value_list)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_value_string'), gt_value_string)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_value_number'), gt_value_number)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_value_bytes'), gt_value_bytes)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_map_set'), gt_map_set)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_map_discard'), gt_map_discard)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_map_clear'), gt_map_clear)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_map_copy_from'), gt_map_copy_from)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_list_push'), gt_list_push)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_list_discard'), gt_list_discard)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_list_clear'), gt_list_clear)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_string_push'), gt_string_push)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_buffer_push'), gt_buffer_push)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_schema_validate'), gt_schema_validate)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_schema_redact'), gt_schema_redact)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_date_parse'), gt_date_parse)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_date_format'), gt_date_format)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_date_normalize'), gt_date_normalize)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_date_add_span'), gt_date_add_span)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_time_parse'), gt_time_parse)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_time_format'), gt_time_format)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_time_normalize'), gt_time_normalize)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_datetime_parse'), gt_datetime_parse)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_datetime_format'), gt_datetime_format)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_datetime_normalize'), gt_datetime_normalize)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_datetime_add_span'), gt_datetime_add_span)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_timestamp_parse'), gt_timestamp_parse)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_timestamp_format'), gt_timestamp_format)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_timestamp_normalize'), gt_timestamp_normalize)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_span_parse'), gt_span_parse)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_span_format'), gt_span_format)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_span_normalize'), gt_span_normalize)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_duration_read'), gt_duration_read)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_new'), gt_registry_new)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_free'), gt_registry_free)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_host'), gt_registry_host)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_keyed_by'), gt_registry_keyed_by)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_libraries_keyed_by'), gt_registry_libraries_keyed_by)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_load_file'), gt_registry_load_file)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_register_entry'), gt_registry_register_entry)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_add_source_file'), gt_registry_add_source_file)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_watch_dir'), gt_registry_watch_dir)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_refresh'), gt_registry_refresh)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_scan_dir'), gt_registry_scan_dir)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_scan_dir_rules'), gt_registry_scan_dir_rules)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_scan_path'), gt_registry_scan_path)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_scan_dir_rules'), gt_registry_scan_dir_rules_split)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_scan_path'), gt_registry_scan_path_split)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_libraries'), gt_registry_libraries)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_providers'), gt_registry_providers)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_provider'), gt_registry_provider)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_available'), gt_registry_available)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_why_not'), gt_registry_why_not)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_best'), gt_registry_best)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_provider_available'), gt_registry_provider_available)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_set_priority'), gt_registry_set_priority)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_priority'), gt_registry_priority)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_set_import_search'), gt_registry_set_import_search)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_import_search'), gt_registry_import_search)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_provider_vtable'), gt_registry_provider_vtable)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_provider_table'), gt_registry_provider_table)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_provider_ctx'), gt_registry_provider_ctx)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_provider_config'), gt_registry_provider_config)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_provider_meta'), gt_registry_provider_meta)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_provider_create'), gt_registry_provider_create)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_provider_destroy'), gt_registry_provider_destroy)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_retire'), gt_registry_retire)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_unload'), gt_registry_unload)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_unload_unchecked'), gt_registry_unload_unchecked)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_registry_collect'), gt_registry_collect)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_wire_encode'), gt_wire_encode)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_wire_decode'), gt_wire_decode)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_wire_announce'), gt_wire_announce)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_wire_send'), gt_wire_send)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_wire_close'), gt_wire_close)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_wire_receiver_new'), gt_wire_receiver_new)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_wire_receiver_accept'), gt_wire_receiver_accept)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_wire_receiver_free'), gt_wire_receiver_free)
  end subroutine

end module guatiao_c
