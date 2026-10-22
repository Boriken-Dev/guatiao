! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

! Opening a shared library and resolving its exports at run time, with
! nothing but iso_c_binding: LoadLibraryW/GetProcAddress on Windows,
! dlopen/dlsym elsewhere. The one file the preprocessor chooses for.

! gfortran predefines no platform macro on Windows; only 64-bit Windows
! is LLP64, so a 4-byte long names it.
#if defined(_WIN32) || (defined(__GFORTRAN__) && __SIZEOF_LONG__ == 4)
#define GT_WINDOWS 1
#endif

module guatiao_loader
  use, intrinsic :: iso_c_binding
  implicit none
  private

  public :: gt_dl_open, gt_dl_sym, gt_dl_error, gt_windows, gt_library_file_names

  !> True when this build was compiled for Windows.
#ifdef GT_WINDOWS
  logical, parameter :: gt_windows = .true.
#else
  logical, parameter :: gt_windows = .false.
#endif

  interface
#ifdef GT_WINDOWS
    function MultiByteToWideChar(cp, flags, src, srclen, dst, dstlen) &
        bind(c, name="MultiByteToWideChar") result(n)
      import :: c_int, c_int32_t, c_char, c_ptr
      integer(c_int32_t), value :: cp, flags
      character(kind=c_char), intent(in) :: src(*)
      integer(c_int), value :: srclen
      type(c_ptr), value :: dst
      integer(c_int), value :: dstlen
      integer(c_int) :: n
    end function
    function LoadLibraryW(path) bind(c, name="LoadLibraryW") result(h)
      import :: c_ptr
      type(c_ptr), value :: path
      type(c_ptr) :: h
    end function
    function GetProcAddress(h, name) bind(c, name="GetProcAddress") result(f)
      import :: c_ptr, c_funptr, c_char
      type(c_ptr), value :: h
      character(kind=c_char), intent(in) :: name(*)
      type(c_funptr) :: f
    end function
    function GetLastError() bind(c, name="GetLastError") result(e)
      import :: c_int32_t
      integer(c_int32_t) :: e
    end function
#else
    function dlopen(path, mode) bind(c, name="dlopen") result(h)
      import :: c_ptr, c_char, c_int
      character(kind=c_char), intent(in) :: path(*)
      integer(c_int), value :: mode
      type(c_ptr) :: h
    end function
    function dlsym(h, name) bind(c, name="dlsym") result(f)
      import :: c_ptr, c_funptr, c_char
      type(c_ptr), value :: h
      character(kind=c_char), intent(in) :: name(*)
      type(c_funptr) :: f
    end function
    function c_dlerror() bind(c, name="dlerror") result(p)
      import :: c_ptr
      type(c_ptr) :: p
    end function
    function c_strlen(p) bind(c, name="strlen") result(n)
      import :: c_ptr, c_size_t
      type(c_ptr), value :: p
      integer(c_size_t) :: n
    end function
#endif
  end interface

contains

  !> Opens the shared library at `path` (UTF-8); a null handle when it
  !> does not open, the reason then in gt_dl_error().
  function gt_dl_open(path) result(h)
    character(*), intent(in) :: path
    type(c_ptr) :: h
#ifdef GT_WINDOWS
    integer(c_int16_t), allocatable, target :: wide(:)
    integer(c_int) :: n
    integer(c_int32_t), parameter :: CP_UTF8 = 65001
    h = c_null_ptr
    if (len(path) == 0) return
    n = MultiByteToWideChar(CP_UTF8, 0_c_int32_t, path, int(len(path), c_int), c_null_ptr, 0_c_int)
    if (n <= 0) return
    allocate(wide(n + 1))
    wide = 0_c_int16_t
    n = MultiByteToWideChar(CP_UTF8, 0_c_int32_t, path, int(len(path), c_int), c_loc(wide), n)
    h = LoadLibraryW(c_loc(wide))
#else
    integer(c_int), parameter :: RTLD_NOW = 2
    h = dlopen(path // c_null_char, RTLD_NOW)
#endif
  end function

  !> The export `name` of an open library, or a null pointer.
  function gt_dl_sym(h, name) result(f)
    type(c_ptr), intent(in) :: h
    character(*), intent(in) :: name
    type(c_funptr) :: f
    f = c_null_funptr
    if (.not. c_associated(h)) return
#ifdef GT_WINDOWS
    f = GetProcAddress(h, name // c_null_char)
#else
    f = dlsym(h, name // c_null_char)
#endif
  end function

  !> Why the last open failed, in the platform's words.
  subroutine gt_dl_error(msg)
    character(:), allocatable, intent(out) :: msg
#ifdef GT_WINDOWS
    character(40) :: buf
    write (buf, '(a,i0)') 'Windows error ', GetLastError()
    msg = trim(buf)
#else
    type(c_ptr) :: p
    character(kind=c_char), pointer :: s(:)
    integer :: n, i
    p = c_dlerror()
    if (.not. c_associated(p)) then
      msg = 'no reason given'
      return
    end if
    n = int(c_strlen(p))
    call c_f_pointer(p, s, [n])
    allocate(character(n) :: msg)
    do i = 1, n
      msg(i:i) = s(i)
    end do
#endif
  end subroutine

  !> The file names a library called `stem` has on this platform, in the
  !> order to try: `guatiao.dll`; `libguatiao.so` then `libguatiao.dylib`.
  subroutine gt_library_file_names(stem, first, second)
    character(*), intent(in) :: stem
    character(:), allocatable, intent(out) :: first, second
#ifdef GT_WINDOWS
    first = stem // '.dll'
    second = ''
#else
    first = 'lib' // stem // '.so'
    second = 'lib' // stem // '.dylib'
#endif
  end subroutine

end module guatiao_loader
