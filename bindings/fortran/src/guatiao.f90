! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> Fortran bindings over the guatiao C ABI: `use guatiao`.
!>
!> Values, statuses, schemas and configurations, the registry, kinds and
!> providers. Documents (guatiao_serde), forms (guatiao_intake) and dates
!> and times (guatiao_civil) are modules of their own.
module guatiao
  use guatiao_c, only: GUATIAO_TAG_ABSENT, GUATIAO_TAG_NULL, GUATIAO_TAG_BOOL, GUATIAO_TAG_NUMBER, &
                       GUATIAO_TAG_STRING, GUATIAO_TAG_BYTES, GUATIAO_TAG_LIST, GUATIAO_TAG_MAP, &
                       GUATIAO_ABI_VERSION, GUATIAO_MAX_DEPTH
  use guatiao_errors
  use guatiao_native, only: guatiao_use_library, guatiao_library_path, guatiao_allocator, &
                            guatiao_default_allocator, GUATIAO_LIB_CORE => GT_CORE, &
                            GUATIAO_LIB_SERDE => GT_SERDE, GUATIAO_LIB_INTAKE => GT_INTAKE
  use guatiao_values, only: guatiao_value, guatiao_text, guatiao_map, guatiao_list, guatiao_null, &
                            guatiao_absent, guatiao_of, guatiao_number, guatiao_value_from_raw
  use guatiao_schema
  implicit none
  public
end module guatiao
