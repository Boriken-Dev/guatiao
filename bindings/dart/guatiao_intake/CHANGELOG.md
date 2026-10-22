# Changelog

All notable changes to this package will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- The package, taken out of package:guatiao, where it was
  `package:guatiao/intake.dart`: the same functions, over the
  `guatiao_intake` library, which it finds through
  `GUATIAO_INTAKE_LIBRARY`, then `GUATIAO_LIBRARY` (a directory,
  or a file exporting its functions, else the file beside it).
