# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

include("${CMAKE_CURRENT_SOURCE_DIR}/../cmake/GuatiaoTests.cmake")

guatiao_cpp_test(cpp_values SOURCES test_values.cpp LIBS guatiao::guatiao BOTH_EXCEPTION_MODES)
guatiao_cpp_test(cpp_vectors SOURCES test_vectors.cpp LIBS guatiao::guatiao BOTH_EXCEPTION_MODES)
guatiao_cpp_test(cpp_frees SOURCES test_frees.cpp LIBS guatiao::guatiao BOTH_EXCEPTION_MODES)
guatiao_cpp_test(cpp_throwing SOURCES test_throwing.cpp LIBS guatiao::guatiao)
guatiao_cpp_test(cpp_number_text SOURCES test_number_text.cpp LIBS guatiao::guatiao BOTH_EXCEPTION_MODES
                 DEFINITIONS GUATIAO_CPP_NO_FLOAT_CHARCONV)
