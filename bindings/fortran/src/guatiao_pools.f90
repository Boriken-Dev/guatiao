! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> Fixed pools of bind(c) procedures for the descriptor slots that carry
!> no ctx naming their provider or library: a provider's `create` (its ctx
!> is null when it builds instances only) and the entry of a library
!> registered in this process. Each passes its index to the target the
!> provide module sets: one procedure per index, GT_POOL_CREATES and
!> GT_POOL_ENTRIES of them.
module guatiao_pools
  use, intrinsic :: iso_c_binding
  implicit none
  private

  public :: GT_POOL_CREATES, GT_POOL_ENTRIES, gt_pool_create_at, gt_pool_entry_at, gt_create_target, &
            gt_entry_target, gt_create_target_i, gt_entry_target_i

  integer, parameter :: GT_POOL_CREATES = 64, GT_POOL_ENTRIES = 16

  abstract interface
    function gt_create_target_i(index, ctx, config, out, err) result(status)
      import :: c_ptr, c_int32_t
      integer, intent(in) :: index
      type(c_ptr), intent(in) :: ctx, config, out, err
      integer(c_int32_t) :: status
    end function
    function gt_entry_target_i(index, host) result(info)
      import :: c_ptr
      integer, intent(in) :: index
      type(c_ptr), intent(in) :: host
      type(c_ptr) :: info
    end function
  end interface

  procedure(gt_create_target_i), pointer :: gt_create_target => null()
  procedure(gt_entry_target_i), pointer :: gt_entry_target => null()

contains

  !> The create slot of pool index `i`.
  function gt_pool_create_at(i) result(f)
    integer, intent(in) :: i
    type(c_funptr) :: f
    select case (i)
    case (1); f = c_funloc(create_01)
    case (2); f = c_funloc(create_02)
    case (3); f = c_funloc(create_03)
    case (4); f = c_funloc(create_04)
    case (5); f = c_funloc(create_05)
    case (6); f = c_funloc(create_06)
    case (7); f = c_funloc(create_07)
    case (8); f = c_funloc(create_08)
    case (9); f = c_funloc(create_09)
    case (10); f = c_funloc(create_10)
    case (11); f = c_funloc(create_11)
    case (12); f = c_funloc(create_12)
    case (13); f = c_funloc(create_13)
    case (14); f = c_funloc(create_14)
    case (15); f = c_funloc(create_15)
    case (16); f = c_funloc(create_16)
    case (17); f = c_funloc(create_17)
    case (18); f = c_funloc(create_18)
    case (19); f = c_funloc(create_19)
    case (20); f = c_funloc(create_20)
    case (21); f = c_funloc(create_21)
    case (22); f = c_funloc(create_22)
    case (23); f = c_funloc(create_23)
    case (24); f = c_funloc(create_24)
    case (25); f = c_funloc(create_25)
    case (26); f = c_funloc(create_26)
    case (27); f = c_funloc(create_27)
    case (28); f = c_funloc(create_28)
    case (29); f = c_funloc(create_29)
    case (30); f = c_funloc(create_30)
    case (31); f = c_funloc(create_31)
    case (32); f = c_funloc(create_32)
    case (33); f = c_funloc(create_33)
    case (34); f = c_funloc(create_34)
    case (35); f = c_funloc(create_35)
    case (36); f = c_funloc(create_36)
    case (37); f = c_funloc(create_37)
    case (38); f = c_funloc(create_38)
    case (39); f = c_funloc(create_39)
    case (40); f = c_funloc(create_40)
    case (41); f = c_funloc(create_41)
    case (42); f = c_funloc(create_42)
    case (43); f = c_funloc(create_43)
    case (44); f = c_funloc(create_44)
    case (45); f = c_funloc(create_45)
    case (46); f = c_funloc(create_46)
    case (47); f = c_funloc(create_47)
    case (48); f = c_funloc(create_48)
    case (49); f = c_funloc(create_49)
    case (50); f = c_funloc(create_50)
    case (51); f = c_funloc(create_51)
    case (52); f = c_funloc(create_52)
    case (53); f = c_funloc(create_53)
    case (54); f = c_funloc(create_54)
    case (55); f = c_funloc(create_55)
    case (56); f = c_funloc(create_56)
    case (57); f = c_funloc(create_57)
    case (58); f = c_funloc(create_58)
    case (59); f = c_funloc(create_59)
    case (60); f = c_funloc(create_60)
    case (61); f = c_funloc(create_61)
    case (62); f = c_funloc(create_62)
    case (63); f = c_funloc(create_63)
    case (64); f = c_funloc(create_64)
    case default; f = c_null_funptr
    end select
  end function

  !> The entry of pool index `i`.
  function gt_pool_entry_at(i) result(f)
    integer, intent(in) :: i
    type(c_funptr) :: f
    select case (i)
    case (1); f = c_funloc(entry_01)
    case (2); f = c_funloc(entry_02)
    case (3); f = c_funloc(entry_03)
    case (4); f = c_funloc(entry_04)
    case (5); f = c_funloc(entry_05)
    case (6); f = c_funloc(entry_06)
    case (7); f = c_funloc(entry_07)
    case (8); f = c_funloc(entry_08)
    case (9); f = c_funloc(entry_09)
    case (10); f = c_funloc(entry_10)
    case (11); f = c_funloc(entry_11)
    case (12); f = c_funloc(entry_12)
    case (13); f = c_funloc(entry_13)
    case (14); f = c_funloc(entry_14)
    case (15); f = c_funloc(entry_15)
    case (16); f = c_funloc(entry_16)
    case default; f = c_null_funptr
    end select
  end function

  recursive function create_01(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(1, ctx, config, out, err)
  end function

  recursive function create_02(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(2, ctx, config, out, err)
  end function

  recursive function create_03(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(3, ctx, config, out, err)
  end function

  recursive function create_04(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(4, ctx, config, out, err)
  end function

  recursive function create_05(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(5, ctx, config, out, err)
  end function

  recursive function create_06(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(6, ctx, config, out, err)
  end function

  recursive function create_07(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(7, ctx, config, out, err)
  end function

  recursive function create_08(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(8, ctx, config, out, err)
  end function

  recursive function create_09(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(9, ctx, config, out, err)
  end function

  recursive function create_10(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(10, ctx, config, out, err)
  end function

  recursive function create_11(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(11, ctx, config, out, err)
  end function

  recursive function create_12(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(12, ctx, config, out, err)
  end function

  recursive function create_13(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(13, ctx, config, out, err)
  end function

  recursive function create_14(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(14, ctx, config, out, err)
  end function

  recursive function create_15(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(15, ctx, config, out, err)
  end function

  recursive function create_16(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(16, ctx, config, out, err)
  end function

  recursive function create_17(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(17, ctx, config, out, err)
  end function

  recursive function create_18(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(18, ctx, config, out, err)
  end function

  recursive function create_19(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(19, ctx, config, out, err)
  end function

  recursive function create_20(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(20, ctx, config, out, err)
  end function

  recursive function create_21(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(21, ctx, config, out, err)
  end function

  recursive function create_22(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(22, ctx, config, out, err)
  end function

  recursive function create_23(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(23, ctx, config, out, err)
  end function

  recursive function create_24(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(24, ctx, config, out, err)
  end function

  recursive function create_25(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(25, ctx, config, out, err)
  end function

  recursive function create_26(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(26, ctx, config, out, err)
  end function

  recursive function create_27(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(27, ctx, config, out, err)
  end function

  recursive function create_28(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(28, ctx, config, out, err)
  end function

  recursive function create_29(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(29, ctx, config, out, err)
  end function

  recursive function create_30(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(30, ctx, config, out, err)
  end function

  recursive function create_31(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(31, ctx, config, out, err)
  end function

  recursive function create_32(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(32, ctx, config, out, err)
  end function

  recursive function create_33(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(33, ctx, config, out, err)
  end function

  recursive function create_34(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(34, ctx, config, out, err)
  end function

  recursive function create_35(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(35, ctx, config, out, err)
  end function

  recursive function create_36(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(36, ctx, config, out, err)
  end function

  recursive function create_37(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(37, ctx, config, out, err)
  end function

  recursive function create_38(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(38, ctx, config, out, err)
  end function

  recursive function create_39(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(39, ctx, config, out, err)
  end function

  recursive function create_40(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(40, ctx, config, out, err)
  end function

  recursive function create_41(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(41, ctx, config, out, err)
  end function

  recursive function create_42(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(42, ctx, config, out, err)
  end function

  recursive function create_43(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(43, ctx, config, out, err)
  end function

  recursive function create_44(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(44, ctx, config, out, err)
  end function

  recursive function create_45(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(45, ctx, config, out, err)
  end function

  recursive function create_46(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(46, ctx, config, out, err)
  end function

  recursive function create_47(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(47, ctx, config, out, err)
  end function

  recursive function create_48(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(48, ctx, config, out, err)
  end function

  recursive function create_49(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(49, ctx, config, out, err)
  end function

  recursive function create_50(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(50, ctx, config, out, err)
  end function

  recursive function create_51(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(51, ctx, config, out, err)
  end function

  recursive function create_52(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(52, ctx, config, out, err)
  end function

  recursive function create_53(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(53, ctx, config, out, err)
  end function

  recursive function create_54(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(54, ctx, config, out, err)
  end function

  recursive function create_55(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(55, ctx, config, out, err)
  end function

  recursive function create_56(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(56, ctx, config, out, err)
  end function

  recursive function create_57(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(57, ctx, config, out, err)
  end function

  recursive function create_58(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(58, ctx, config, out, err)
  end function

  recursive function create_59(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(59, ctx, config, out, err)
  end function

  recursive function create_60(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(60, ctx, config, out, err)
  end function

  recursive function create_61(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(61, ctx, config, out, err)
  end function

  recursive function create_62(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(62, ctx, config, out, err)
  end function

  recursive function create_63(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(63, ctx, config, out, err)
  end function

  recursive function create_64(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    status = gt_create_target(64, ctx, config, out, err)
  end function

  recursive function entry_01(host) bind(c) result(info)
    type(c_ptr), value :: host
    type(c_ptr) :: info
    info = gt_entry_target(1, host)
  end function

  recursive function entry_02(host) bind(c) result(info)
    type(c_ptr), value :: host
    type(c_ptr) :: info
    info = gt_entry_target(2, host)
  end function

  recursive function entry_03(host) bind(c) result(info)
    type(c_ptr), value :: host
    type(c_ptr) :: info
    info = gt_entry_target(3, host)
  end function

  recursive function entry_04(host) bind(c) result(info)
    type(c_ptr), value :: host
    type(c_ptr) :: info
    info = gt_entry_target(4, host)
  end function

  recursive function entry_05(host) bind(c) result(info)
    type(c_ptr), value :: host
    type(c_ptr) :: info
    info = gt_entry_target(5, host)
  end function

  recursive function entry_06(host) bind(c) result(info)
    type(c_ptr), value :: host
    type(c_ptr) :: info
    info = gt_entry_target(6, host)
  end function

  recursive function entry_07(host) bind(c) result(info)
    type(c_ptr), value :: host
    type(c_ptr) :: info
    info = gt_entry_target(7, host)
  end function

  recursive function entry_08(host) bind(c) result(info)
    type(c_ptr), value :: host
    type(c_ptr) :: info
    info = gt_entry_target(8, host)
  end function

  recursive function entry_09(host) bind(c) result(info)
    type(c_ptr), value :: host
    type(c_ptr) :: info
    info = gt_entry_target(9, host)
  end function

  recursive function entry_10(host) bind(c) result(info)
    type(c_ptr), value :: host
    type(c_ptr) :: info
    info = gt_entry_target(10, host)
  end function

  recursive function entry_11(host) bind(c) result(info)
    type(c_ptr), value :: host
    type(c_ptr) :: info
    info = gt_entry_target(11, host)
  end function

  recursive function entry_12(host) bind(c) result(info)
    type(c_ptr), value :: host
    type(c_ptr) :: info
    info = gt_entry_target(12, host)
  end function

  recursive function entry_13(host) bind(c) result(info)
    type(c_ptr), value :: host
    type(c_ptr) :: info
    info = gt_entry_target(13, host)
  end function

  recursive function entry_14(host) bind(c) result(info)
    type(c_ptr), value :: host
    type(c_ptr) :: info
    info = gt_entry_target(14, host)
  end function

  recursive function entry_15(host) bind(c) result(info)
    type(c_ptr), value :: host
    type(c_ptr) :: info
    info = gt_entry_target(15, host)
  end function

  recursive function entry_16(host) bind(c) result(info)
    type(c_ptr), value :: host
    type(c_ptr) :: info
    info = gt_entry_target(16, host)
  end function

end module guatiao_pools
