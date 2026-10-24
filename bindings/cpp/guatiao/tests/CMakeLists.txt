# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

include("${CMAKE_CURRENT_SOURCE_DIR}/../cmake/GuatiaoTests.cmake")

guatiao_cpp_test(cpp_values SOURCES test_values.cpp LIBS guatiao::guatiao BOTH_EXCEPTION_MODES)
guatiao_cpp_test(cpp_vectors SOURCES test_vectors.cpp LIBS guatiao::guatiao BOTH_EXCEPTION_MODES)
guatiao_cpp_test(cpp_frees SOURCES test_frees.cpp LIBS guatiao::guatiao BOTH_EXCEPTION_MODES)
guatiao_cpp_test(cpp_throwing SOURCES test_throwing.cpp LIBS guatiao::guatiao
                 INCLUDES "${GUATIAO_REPO}/examples/greeter_kind/include" "${GUATIAO_REPO}/examples/greeter_kind/cpp")
guatiao_cpp_test(cpp_kinds SOURCES test_kinds.cpp LIBS guatiao::guatiao Threads::Threads BOTH_EXCEPTION_MODES
                 INCLUDES "${GUATIAO_REPO}/examples/greeter_kind/include" "${GUATIAO_REPO}/examples/greeter_kind/cpp")
set(kinds_include "${GUATIAO_REPO}/examples/greeter_kind/include" "${GUATIAO_REPO}/examples/greeter_kind/cpp")
guatiao_cpp_compile_fail(cpp_refuses_a_misdeclared_method compile_fail/misdeclared_method.cpp
                         "does not fit its slot" LIBS guatiao::guatiao INCLUDES ${kinds_include})
guatiao_cpp_compile_fail(cpp_refuses_a_missing_required_slot compile_fail/missing_required.cpp
                         "is a required slot" LIBS guatiao::guatiao INCLUDES ${kinds_include})
guatiao_cpp_compile_fail(cpp_refuses_a_kind_against_its_table compile_fail/kind_against_its_table.cpp
                         "does not stand for its field" LIBS guatiao::guatiao INCLUDES ${kinds_include})
guatiao_cpp_compile_fail(cpp_refuses_a_view_of_a_temporary compile_fail/temporary_as_map.cpp
                         "would dangle" LIBS guatiao::guatiao)
guatiao_cpp_compile_fail(cpp_refuses_a_pointer_as_an_item compile_fail/pointer_item.cpp
                         "deleted" LIBS guatiao::guatiao)
guatiao_cpp_compile_fail(cpp_refuses_an_argument_the_slot_does_not_take compile_fail/pointer_argument.cpp
                         "not one the slot takes" LIBS guatiao::guatiao INCLUDES ${kinds_include})
guatiao_cpp_test(cpp_number_text SOURCES test_number_text.cpp LIBS guatiao::guatiao BOTH_EXCEPTION_MODES
                 DEFINITIONS GUATIAO_CPP_NO_FLOAT_CHARCONV)
guatiao_check_kinds(cpp_kinds_agree_with_their_description
                    HEADER "${GUATIAO_REPO}/examples/greeter_kind/cpp/greeter_kind.hpp"
                    DESCRIPTION "${GUATIAO_REPO}/examples/greeter_kind/include/greeter_kind.kinds.json")
guatiao_check_kinds(cpp_kind_check_refuses_a_lent_tree_declared_taken
                    HEADER "${CMAKE_CURRENT_SOURCE_DIR}/kind_check/lent_declared_taken.hpp"
                    DESCRIPTION "${GUATIAO_REPO}/examples/greeter_kind/include/greeter_kind.kinds.json"
                    EXPECT "greeter.check: declared bool\\(map\\), described bool\\(map_ref\\)")
guatiao_check_kinds(cpp_kind_check_reads_a_named_kind_past_a_comment
                    HEADER kind_check/named_after_a_comment.hpp
                    DESCRIPTION "${GUATIAO_REPO}/examples/greeter_kind/include/greeter_kind.kinds.json"
                    EXPECT "tally.count: declared void\\(\\), described int64_t\\(\\)")
