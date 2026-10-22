# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# The example host runs as its README says, in a process of its own.

case("the R host example runs", with_examples({
  host <- repo_file("examples", "r_host", "host.R")
  args <- LIBRARY
  c_dir <- build_c_greeter()
  if (!is.null(c_dir)) args <- c(args, c_dir)
  go <- Sys.getenv("GUATIAO_GO_LIBRARY")
  if (nzchar(go)) args <- c(args, go)
  py <- bridge()
  if (!is.null(py)) {
    args <- c(args, "--python-source", py, "--python-dir", repo_file("examples", "python_provider"),
      "--python-path", repo_file("bindings", "python", "guatiao", "src"),
      "--python-path", repo_file("examples", "greeter_kind", "python"))
  }
  rscript <- file.path(R.home("bin"), if (.Platform$OS.type == "windows") "Rscript.exe" else "Rscript")
  rlibs <- paste(.libPaths(), collapse = .Platform$path.sep)
  out <- withr_env(sprintf("R_LIBS=%s", rlibs), system2(rscript, c(shQuote(host), shQuote(args)), stdout = TRUE, stderr = FALSE))
  text <- paste(out, collapse = "\n")
  has <- function(s) expect_true(grepl(s, text, fixed = TRUE), s)
  has("registered rgreeter 1.0.0 from R")
  has("rgreeter_hello (rgreeter 1.0.0)\n  greet: hello, ana")
  has("derived_greeter_hello")
  has("an empty configuration is refused")
  has("not this greeter kind") # hello_library's table is another kind's
  has("a conversation of 2 turns: \"hi\\nagain\"")
  has("closed it; the listener heard hi, again and was let go")
  if (!is.null(c_dir)) has("c_greeter (c_greeter_library 1.0)\n  greet: hello from C, ana")
  if (nzchar(go)) has("go_greeter_hello")
  if (!is.null(py)) has("served py_greeter from Python")
  has("retired rgreeter, still mapped: unload said WrongKind")
  expect_true(endsWith(text, "objects a library can still reach: 0"))
}))
