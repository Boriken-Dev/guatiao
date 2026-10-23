# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

include("${CMAKE_CURRENT_SOURCE_DIR}/../../guatiao/cmake/GuatiaoTests.cmake")

guatiao_cpp_test(cpp_intake SOURCES test_intake.cpp LIBS guatiao::intake BOTH_EXCEPTION_MODES)
