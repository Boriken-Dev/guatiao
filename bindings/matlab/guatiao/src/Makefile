# Builds guatiao_mex.mex when Octave's pkg installs the package. MKOCTFILE
# is set by pkg; libffi comes from pkg-config where it answers, else -lffi.
# -fexceptions: a MEX error unwinds through the gateway's C frames as a C++
# exception, which needs unwind tables gcc does not give C on every target.

MKOCTFILE ?= mkoctfile
FFI_CFLAGS := $(shell pkg-config --cflags libffi 2>/dev/null)
FFI_LIBS := $(shell pkg-config --libs libffi 2>/dev/null)
ifeq ($(strip $(FFI_LIBS)),)
FFI_LIBS := -lffi
endif
ifeq ($(shell uname -s 2>/dev/null),Darwin)
SDK := $(shell xcrun --show-sdk-path 2>/dev/null)
ifneq ($(SDK),)
ifeq ($(strip $(FFI_CFLAGS)),)
FFI_CFLAGS := -I$(SDK)/usr/include/ffi
endif
endif
endif
ifeq ($(shell uname -s 2>/dev/null),Linux)
OS_LIBS := -ldl -lpthread
endif

SOURCES := $(wildcard *.c)

guatiao_mex.mex: $(SOURCES) $(wildcard *.h)
	$(MKOCTFILE) --mex -o $@ -fexceptions -I. $(FFI_CFLAGS) $(SOURCES) $(FFI_LIBS) $(OS_LIBS)

clean:
	rm -f guatiao_mex.mex *.o

.PHONY: clean
