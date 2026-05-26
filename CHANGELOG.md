# Changelog

## [0.1.16](https://github.com/michalschroeder/ci-tui/compare/v0.1.15...v0.1.16) (2026-05-26)


### Bug Fixes

* **deps:** update rust crate sysinfo to 0.39 ([#71](https://github.com/michalschroeder/ci-tui/issues/71)) ([14c57ba](https://github.com/michalschroeder/ci-tui/commit/14c57bacc746034eb0dd02742e681eaee9aa5d43))


### Code Refactoring

* route fix.rs + simple.rs through CommandExecutor ([#68](https://github.com/michalschroeder/ci-tui/issues/68)) ([82addf1](https://github.com/michalschroeder/ci-tui/commit/82addf1b7fa3c7267a16bddb85bdccb35616aa15))
* **test_discovery:** inject ProcessRunner into grep_search ([#70](https://github.com/michalschroeder/ci-tui/issues/70)) ([5fd6b4a](https://github.com/michalschroeder/ci-tui/commit/5fd6b4a82dea89781db28799b3f12b5481cf880b))

## [0.1.15](https://github.com/michalschroeder/ci-tui/compare/v0.1.14...v0.1.15) (2026-04-23)


### Features

* add host mode for pre-commands ([#60](https://github.com/michalschroeder/ci-tui/issues/60)) ([e605121](https://github.com/michalschroeder/ci-tui/commit/e605121b64682d0cb09751cabeaac42e07854fc7))
* **config:** validate regex at load + deterministic pattern handling ([#63](https://github.com/michalschroeder/ci-tui/issues/63)) ([0ba872d](https://github.com/michalschroeder/ci-tui/commit/0ba872db2d0e9a66ef68364380d51a4ce7509d8f))
* **git:** include untracked files in change detection ([#64](https://github.com/michalschroeder/ci-tui/issues/64)) ([f450433](https://github.com/michalschroeder/ci-tui/commit/f450433895238e4c8867e9e0c7e1f774ee636c2f))


### Bug Fixes

* **git:** correct diff semantics for changed-file detection ([#62](https://github.com/michalschroeder/ci-tui/issues/62)) ([935b797](https://github.com/michalschroeder/ci-tui/commit/935b797313c68337e6773af2ab52412b6a13ac04))


### Documentation

* spec for correct git diff semantics ([fcb4d07](https://github.com/michalschroeder/ci-tui/commit/fcb4d0746476a830afc74cab20a885d29caaf868))

## [0.1.14](https://github.com/michalschroeder/ci-tui/compare/v0.1.13...v0.1.14) (2026-03-19)


### Features

* add --files CLI arg to bypass git change detection ([fc3666a](https://github.com/michalschroeder/ci-tui/commit/fc3666a0b08447662da8a3028e8906cc5bfe3650))
* add --files CLI arg to bypass git detection ([#47](https://github.com/michalschroeder/ci-tui/issues/47)) ([850d605](https://github.com/michalschroeder/ci-tui/commit/850d605fb62823e55d190a5d0cd6ce4545592e1e))


### Documentation

* update CLAUDE.md architecture section ([fe2508d](https://github.com/michalschroeder/ci-tui/commit/fe2508d0bc0adae54dba5111a4468e2975f502e7))
* update CLAUDE.md architecture section ([#49](https://github.com/michalschroeder/ci-tui/issues/49)) ([acdddbe](https://github.com/michalschroeder/ci-tui/commit/acdddbe1cd5712ae41c04f4bc0968257d70cf039))

## [0.1.13](https://github.com/michalschroeder/ci-tui/compare/v0.1.12...v0.1.13) (2026-02-10)


### Features

* **07-01:** create utils module infrastructure with time formatting ([5880941](https://github.com/michalschroeder/ci-tui/commit/5880941c445a36343652b0051a3fba4a0ae75e6f))


### Bug Fixes

* **08-01:** resolve excessive nesting and lint priority issues ([ca2f911](https://github.com/michalschroeder/ci-tui/commit/ca2f91125072cc23c4785c92c929e71381595376))
* **08-02:** reduce process_triggered_check parameter count ([fb73dba](https://github.com/michalschroeder/ci-tui/commit/fb73dbac1f43051cf219cbab12fdd0844c60fe51))
* **ui:** restore terminal using backend's stdout handle before printing summary ([bca160b](https://github.com/michalschroeder/ci-tui/commit/bca160b1d4a77b8ac1188761f31ca335975f0938))


### Code Refactoring

* **07-01:** migrate dashboard.rs to use utils::time::format ([c375557](https://github.com/michalschroeder/ci-tui/commit/c375557a001451fcf73901bd8dca56f33b3ae5ca))
* **07-02:** migrate docker utilities to utils module ([f5aadf3](https://github.com/michalschroeder/ci-tui/commit/f5aadf30b73325fec4e750410d2a922622dafa85))
* **07-02:** migrate time formatting to utils module ([91e7df1](https://github.com/michalschroeder/ci-tui/commit/91e7df175b7efe09758e685b2e759d79861c27b7))
* **08-02:** extract helper functions from determine_checks() ([ae60421](https://github.com/michalschroeder/ci-tui/commit/ae60421674945aa785449aabf1f9e69525374042))
* fix clippy issues ([b0e4828](https://github.com/michalschroeder/ci-tui/commit/b0e4828a4b2443931e577fdf8eee58ffa7ee53d9))
* **simple:** show full checks output in simple mode ([a77d5bb](https://github.com/michalschroeder/ci-tui/commit/a77d5bb2953a865b41ba92b85b6fc1d6dd98814c))


### Documentation

* **07-02:** complete code deduplication plan ([f8b2f21](https://github.com/michalschroeder/ci-tui/commit/f8b2f2131f4fcb910b3f52a5cbacad3e7f39c5c6))
* **08-01:** complete complexity lints and characterization tests plan ([d5df03d](https://github.com/michalschroeder/ci-tui/commit/d5df03de2bd915a76cdfaae05ad5c1215804d848))
* **08-02:** complete code deduplication plan ([035e3eb](https://github.com/michalschroeder/ci-tui/commit/035e3eb69f37d118d6285e370bab4c70784b3197))
* **09-02:** update project state after simple/fix test coverage ([cd1e92d](https://github.com/michalschroeder/ci-tui/commit/cd1e92dd5587ecae3576014424c02f4512287968))
* **09:** update roadmap with Phase 9 plan structure ([5374fc3](https://github.com/michalschroeder/ci-tui/commit/5374fc3acc5ca0969214ef46120c97cd523f5591))
* change time format example to `ignore` code block ([a0e6bee](https://github.com/michalschroeder/ci-tui/commit/a0e6beeabc49d3a6c52fce206babdb32101e1998))
* **phase-7:** complete code deduplication phase ([80085fa](https://github.com/michalschroeder/ci-tui/commit/80085fae522896d701c5a808aa1c0653f0f37acb))
* **phase-8:** complete complexity reduction phase ([1936a8e](https://github.com/michalschroeder/ci-tui/commit/1936a8e7b7d5e16782d2a9975fe165ccde315599))
* **phase-9:** complete test coverage expansion phase ([78c1dea](https://github.com/michalschroeder/ci-tui/commit/78c1dea47f61b13f4f9c292dde0361325d67c00e))
* update CLAUDE.md validation workflow with make targets ([65bd075](https://github.com/michalschroeder/ci-tui/commit/65bd0757d73f19a112ce0d5fae4a6b6cf74dfa73))

## [0.1.12](https://github.com/michalschroeder/ci-tui/compare/v0.1.11...v0.1.12) (2026-01-29)


### Features

* **06-01:** create ConfigBuilder and CheckBuilder test fixtures ([fa91046](https://github.com/michalschroeder/ci-tui/commit/fa91046d299d3f1c260b41529d5047d4d92a8cba))
* **06-02:** migrate config.rs tests to use inline ConfigBuilder ([bc029fe](https://github.com/michalschroeder/ci-tui/commit/bc029fe38241fcfdf7c3ed6a075538855aa8df26))
* **06-03:** migrate checks.rs tests to structured config fixture ([a94f1d5](https://github.com/michalschroeder/ci-tui/commit/a94f1d5e5675baf68491483fc48527e94f7f2e28))
* **06-03:** migrate widget tests to use fixture ([dd62279](https://github.com/michalschroeder/ci-tui/commit/dd622795b0ed259ebc4116ba9e1db9997838f3f0))


### Bug Fixes

* **06-02:** make compiled_ignore_patterns accessible to tests ([df6355e](https://github.com/michalschroeder/ci-tui/commit/df6355e40f993177f10b49e33ff2a0477d599cb9))
* add CiConfig constructor for test fixture compatibility ([dd737c6](https://github.com/michalschroeder/ci-tui/commit/dd737c667a9d52216a8ddbdce23c343d89a45043))
* config tui ([8f67a2d](https://github.com/michalschroeder/ci-tui/commit/8f67a2d4700b4e1d2a76650123e09625474b1591))
* **test:** suppress unused import warning in common module ([e047029](https://github.com/michalschroeder/ci-tui/commit/e047029776364933454df8aac68bf9414c7cd82a))


### Code Refactoring

* **test:** re-export runner types in common module ([12e40eb](https://github.com/michalschroeder/ci-tui/commit/12e40eb1cc04d5c4c53b5902f0a63853e6f0a812))


### Documentation

* **06-01:** document test fixture usage in CLAUDE.md ([18b2203](https://github.com/michalschroeder/ci-tui/commit/18b22039b00d938f790b8bb908b3d3d315601f2e))
* **06-02:** complete plan 06-02 summary and state update ([42c6572](https://github.com/michalschroeder/ci-tui/commit/42c65728aa0e347641590972d50fa00a3a170614))
* **06-03:** complete Phase 6 with SUMMARY and STATE updates ([3db4498](https://github.com/michalschroeder/ci-tui/commit/3db4498a798c8349124ca787910b5e3b129d7515))
* **06-04:** add edge case comments to checks.rs inline YAML tests ([a0877c3](https://github.com/michalschroeder/ci-tui/commit/a0877c305a5618df085322c75edd7e2a696c7792))
* **06-04:** add edge case comments to config.rs inline YAML tests ([33440a3](https://github.com/michalschroeder/ci-tui/commit/33440a3a3c1ae75e32d0159900d0de402f37667a))
* **06-04:** document test fixture architecture in CLAUDE.md ([0cec43b](https://github.com/michalschroeder/ci-tui/commit/0cec43be89fc483413192073e5d33776bb8fe41b))
* **06:** complete test infrastructure consolidation phase ([8c7fe53](https://github.com/michalschroeder/ci-tui/commit/8c7fe53475e547c3d349f65c07a7df8df699525e))
* **06:** update roadmap for gap closure plan ([85d5746](https://github.com/michalschroeder/ci-tui/commit/85d574670c4499dd6e4018b9e2cc9fc9856eacd2))
* **06:** update roadmap with Phase 6 plans ([8ed14ac](https://github.com/michalschroeder/ci-tui/commit/8ed14ac4dc73bb8163136fa8f62f9ef36fd18aff))
* complete v2.0 cleanup research ([0b87b80](https://github.com/michalschroeder/ci-tui/commit/0b87b80f59b35a6a88a9b0e810cabd1973d6158f))
* create milestone v2.0 roadmap (4 phases) ([f4a40a3](https://github.com/michalschroeder/ci-tui/commit/f4a40a385f3c0039bac190e6051e2d886ae0cab7))
* define milestone v2.0 requirements ([ab3d24a](https://github.com/michalschroeder/ci-tui/commit/ab3d24a80f2e68da06c02d2bdca7906a6a3f195a))
* start milestone v2.0 Comprehensive Cleanup ([b5e8634](https://github.com/michalschroeder/ci-tui/commit/b5e8634cc5fa65e069a8b7a6754cb4480151252b))

## [0.1.11](https://github.com/michalschroeder/ci-tui/compare/v0.1.10...v0.1.11) (2026-01-28)


### Features

* add configurable shell option for Alpine container compatibility ([b453fcd](https://github.com/michalschroeder/ci-tui/commit/b453fcdb1e3a3094114b3795223eb5497919d2a2))
* add configurable shell option for Alpine container compatibility ([#28](https://github.com/michalschroeder/ci-tui/issues/28)) ([0c84689](https://github.com/michalschroeder/ci-tui/commit/0c846895ddbb730b3c9fe372ee61c5547cacc269))

## [0.1.10](https://github.com/michalschroeder/ci-tui/compare/v0.1.9...v0.1.10) (2026-01-25)


### Documentation

* add Release Please versioning section to CLAUDE.md ([7558f9f](https://github.com/michalschroeder/ci-tui/commit/7558f9fd59cf0a9f68e50c76573a9e6e26de4223))

## [0.1.9](https://github.com/michalschroeder/ci-tui/compare/v0.1.8...v0.1.9) (2026-01-25)


### Features

* **04-02:** add CommandExecutor trait and refactor runner.rs ([fc2c933](https://github.com/michalschroeder/ci-tui/commit/fc2c933e7a11ccd835657fa34f3bb30deb595e76))
* **05-01:** add AppMessage enum and App::update() method ([19b8e0f](https://github.com/michalschroeder/ci-tui/commit/19b8e0f95207686751195ef9574ce0f0cadd4d15))
* **05-02:** add test helper for creating App with known state ([81c23af](https://github.com/michalschroeder/ci-tui/commit/81c23af6fa19958dd4a4bc7ffdf1d10c175a9ce3))
* **05-04:** make dashboard module public for widget tests ([8b1717f](https://github.com/michalschroeder/ci-tui/commit/8b1717f1af0e7ee522e98e7b6a9e791947ff89a5))


### Bug Fixes

* **05-03:** fix test compilation errors ([d62f99e](https://github.com/michalschroeder/ci-tui/commit/d62f99e672360d4d1c0d84834118b798540800d7))
* **05-03:** make app module public and fix test dead_code warnings ([b56796b](https://github.com/michalschroeder/ci-tui/commit/b56796baab02cdd47cc0aab34c8f051cd30aeb5a))
* **05-04:** remove unused import and fix deprecated buffer.get() calls ([fad5b9b](https://github.com/michalschroeder/ci-tui/commit/fad5b9b39d46f2726e0170ca7b3c11ad9f7fc848))
* **05-05:** fix widget test to check all checkmarks not just first ([3a50070](https://github.com/michalschroeder/ci-tui/commit/3a5007031f6497516c7fe684d3b51714dc4cbf08))
* **deps:** update rust crate sysinfo to 0.38 ([2039095](https://github.com/michalschroeder/ci-tui/commit/20390955fde64f1a7a1d86e334713ea32559439f))
* **deps:** update rust crate sysinfo to 0.38 ([#22](https://github.com/michalschroeder/ci-tui/issues/22)) ([e540d00](https://github.com/michalschroeder/ci-tui/commit/e540d00bd90b55341e446f09f29b2db4fd88e97d))


### Code Refactoring

* **04-01:** add GitExecutor trait and refactor git.rs ([999db00](https://github.com/michalschroeder/ci-tui/commit/999db00cd2e642282f93ce3a0063afa98b3a4816))
* **05-01:** use AppMessage dispatch in mod.rs event loop ([08c986e](https://github.com/michalschroeder/ci-tui/commit/08c986ee26aaf848eb3503251a894c00e096e42f))


### Documentation

* **04-02:** complete CommandExecutor trait plan ([fafd0d5](https://github.com/michalschroeder/ci-tui/commit/fafd0d572d239cfff0806e7f725e5963404c0046))
* **05-01:** complete TEA-lite state management plan ([382f53d](https://github.com/michalschroeder/ci-tui/commit/382f53db8bdc620e6a341dde93f5dbcf58a94308))
* **05-03:** complete panic hook verification and coverage milestone ([e1b74d1](https://github.com/michalschroeder/ci-tui/commit/e1b74d1cc44a4155d714a4210c78f7b600c57eef))
* **05-03:** mark Phase 5 as complete ([161bbc0](https://github.com/michalschroeder/ci-tui/commit/161bbc02b75cf174078e32672fd708218c570115))
* **05-04:** complete widget tests plan ([2036014](https://github.com/michalschroeder/ci-tui/commit/203601454e245755282d585d94a271652bf01374))
* **05-05:** generate HTML coverage report ([0f005c7](https://github.com/michalschroeder/ci-tui/commit/0f005c7cfc4b5a7a58dc18c2a247c11ed639b789))
* **05-05:** update STATE.md with plan 05-05 completion ([f9f8c79](https://github.com/michalschroeder/ci-tui/commit/f9f8c7902b1300591e6c2d5224086451a6bc20b4))
* **05:** complete Widget Tests & Architecture phase ([7980c98](https://github.com/michalschroeder/ci-tui/commit/7980c9875b0020da7a416b16ad23618d32ca498b))
* **phase-4:** complete mock-based-tests phase ([8fab84d](https://github.com/michalschroeder/ci-tui/commit/8fab84d6f1df5a03b8dc5d95291ba1dcfa87e84c))
* remove unnecessary `-it` flags from Docker commands in CLAUDE.md ([be60d64](https://github.com/michalschroeder/ci-tui/commit/be60d640104fed8cb013a5dcaaa795c27b17346c))

## [0.1.8](https://github.com/michalschroeder/ci-tui/compare/v0.1.7...v0.1.8) (2026-01-24)


### Features

* **02-01:** configure Clippy lints in Cargo.toml ([8745713](https://github.com/michalschroeder/ci-tui/commit/87457139ea0c675111edfd65a5151b489e2ead40))
* **quick-021:** replace Laravel example with Symfony example ([b0f55e3](https://github.com/michalschroeder/ci-tui/commit/b0f55e3b4416b44139add6db9eaa9d92ad169c97))


### Bug Fixes

* **02-02:** replace production unwrap calls with proper Option handling ([e8e8d2f](https://github.com/michalschroeder/ci-tui/commit/e8e8d2fee5a5d17311e563f04db8c5b8548c6f6d))
* **test-discovery:** add missing path argument to grep command ([b62ce7c](https://github.com/michalschroeder/ci-tui/commit/b62ce7cae829e789bc256e6b3087876f3dce0cbf))
* **test-discovery:** strip ./ prefix from grep output and fix test conflicts ([a84061f](https://github.com/michalschroeder/ci-tui/commit/a84061fa6b6be5aa5398f79a41147f2ce67e496c))


### Documentation

* **02-02:** add comprehensive module and public API documentation ([4f4f8cf](https://github.com/michalschroeder/ci-tui/commit/4f4f8cf8c8997eacd5463806f11d59bfb3563315))
* **03-01:** complete checks.rs unit test coverage plan ([c32f319](https://github.com/michalschroeder/ci-tui/commit/c32f319ffe234e8b88561ae2fd317b09233d190a))
* **phase-2:** complete Code Quality Baseline phase ([3b9e3eb](https://github.com/michalschroeder/ci-tui/commit/3b9e3ebc83c0e5cdb6e16be1030409ad44a65390))
* **phase-3:** complete Unit Test Coverage phase ([9f60d96](https://github.com/michalschroeder/ci-tui/commit/9f60d96af78ef808503e9dce9b196a6f656ebf71))
* **quick-020:** complete config documentation plan ([2f3a63b](https://github.com/michalschroeder/ci-tui/commit/2f3a63bb19b0cf4fc14ef5516a56faf42247c7cc))
* **quick-020:** create comprehensive configuration reference ([acc7562](https://github.com/michalschroeder/ci-tui/commit/acc7562eec5145c7f2387c8f73580750deff3d15))
* **quick-020:** create PHP Laravel example config ([f2a276b](https://github.com/michalschroeder/ci-tui/commit/f2a276b04a3a73a859aef8a03d47620c5bec4935))
* **quick-021:** complete Replace Laravel example with Symfony example task ([d6e923f](https://github.com/michalschroeder/ci-tui/commit/d6e923f949194815c5fa3c5c2c06d4db8de9b339))
* **quick-021:** update documentation to reference Symfony example ([e760c38](https://github.com/michalschroeder/ci-tui/commit/e760c381f584e2f58ef45bad4486b8e342681773))
* **quick-022:** add grep_search tests for test discovery ([d0fff92](https://github.com/michalschroeder/ci-tui/commit/d0fff928dfeeb609c75ea8c9a37648d86177fcc1))
* **quick-023:** fix grep_search missing path argument ([99f7188](https://github.com/michalschroeder/ci-tui/commit/99f71880b347a1d5f6e38dbe7256fa2180a036f4))

## [0.1.7](https://github.com/michalschroeder/ci-tui/compare/v0.1.6...v0.1.7) (2026-01-23)


### Features

* **quick-018:** deduplicate matched files in determine_checks ([5544e71](https://github.com/michalschroeder/ci-tui/commit/5544e71af8f204dd6ccfaa03e6d46fe2383fbe93))
* **quick-019:** add --fix CLI argument and fix mode module ([c4c47f5](https://github.com/michalschroeder/ci-tui/commit/c4c47f5a0ff49fd82291d45d1219a5bd45151bc4))


### Documentation

* **CLAUDE:** add --fix workflow and failure handling instructions ([13842c3](https://github.com/michalschroeder/ci-tui/commit/13842c3d33cc41a999d093e938fd3c23ab22acb8))
* **CLAUDE:** update validation instructions to use ci-tui ([7c46cd1](https://github.com/michalschroeder/ci-tui/commit/7c46cd195a7b81f6092324676957ef0627318049))
* **quick-018:** complete TDD test for duplicate detection task ([06ded34](https://github.com/michalschroeder/ci-tui/commit/06ded34b38d24e23855ca1dbc4a0d1efb2d97830))
* **quick-019:** complete add --fix parameter task ([145bc72](https://github.com/michalschroeder/ci-tui/commit/145bc7243215e7648f08a38c1579a25f0a954ef2))

## [0.1.6](https://github.com/michalschroeder/ci-tui/compare/v0.1.5...v0.1.6) (2026-01-23)


### Bug Fixes

* **checks:** skip test discovery fallback when {files} placeholder used ([f01005f](https://github.com/michalschroeder/ci-tui/commit/f01005f67ac5a83249968ff6be69de41770f9f52))

## [0.1.5](https://github.com/michalschroeder/ci-tui/compare/v0.1.4...v0.1.5) (2026-01-23)


### Performance Improvements

* **ci:** optimize Docker builds with native ARM64 runners ([2c9be2a](https://github.com/michalschroeder/ci-tui/commit/2c9be2ae3a8987a9b105bf423e122617a94ddd6f))
* **ci:** remove Docker build from CI workflow ([665ce77](https://github.com/michalschroeder/ci-tui/commit/665ce77ebbf3bd2f4e319ec05ee443c55efa446d))


### Documentation

* **quick-017:** update STATE.md for Docker build optimization ([6a9f4e6](https://github.com/michalschroeder/ci-tui/commit/6a9f4e61d8d88275e2ce4441e3a260641674a240))

## [0.1.4](https://github.com/michalschroeder/ci-tui/compare/v0.1.3...v0.1.4) (2026-01-23)


### Features

* **quick-016:** add skipped_no_files detection in checks.rs ([5ff2006](https://github.com/michalschroeder/ci-tui/commit/5ff20061ae9711f6d7276dc50c5c955db0c551c6))
* **quick-016:** initialize skipped checks in UI with proper status ([eec3e13](https://github.com/michalschroeder/ci-tui/commit/eec3e13593dff175176d3409b709f90b671295b2))


### Bug Fixes

* **ci:** chain release workflow from release-please ([186e682](https://github.com/michalschroeder/ci-tui/commit/186e68261f83feebd716fba279c8452b36254cb0))


### Documentation

* **quick-016:** update STATE.md after completing quick task 016 ([7f00380](https://github.com/michalschroeder/ci-tui/commit/7f00380dd22d671e037f69a9bcf27d8b4e0d16e0))

## [0.1.3](https://github.com/michalschroeder/ci-tui/compare/v0.1.2...v0.1.3) (2026-01-23)


### Bug Fixes

* **ci:** add checkout and explicit token to release-please ([bf29607](https://github.com/michalschroeder/ci-tui/commit/bf296073d22689bedc71427cb489af92b19a7c31))
* **ci:** trigger release build on GitHub release event ([3f96b58](https://github.com/michalschroeder/ci-tui/commit/3f96b58af2885e547b1da34ae10be34b7bc52b70))

## [0.1.2](https://github.com/michalschroeder/ci-tui/compare/v0.1.1...v0.1.2) (2026-01-23)


### Bug Fixes

* **ci:** trigger release-please only after CI passes ([4d2cf04](https://github.com/michalschroeder/ci-tui/commit/4d2cf04c2dae5d652cbcff3c0be4a5ad19e36def))

## [0.1.1](https://github.com/michalschroeder/ci-tui/compare/v0.1.0...v0.1.1) (2026-01-23)


### Features

* **002:** display version in TUI footer ([dc4016f](https://github.com/michalschroeder/ci-tui/commit/dc4016f1ce66065b73f0442bacb589e6d955e6a7))
* **01-03:** create GitHub Actions CI workflow ([c87b548](https://github.com/michalschroeder/ci-tui/commit/c87b5488c8c2056c773cdb9d9403f4737961b4e9))
* **config:** add per-check container override support ([5498182](https://github.com/michalschroeder/ci-tui/commit/54981826f1a9a3097fc591daa7eaf03213ec7209))
* **config:** expand environment variables in volume_mount ([738fc49](https://github.com/michalschroeder/ci-tui/commit/738fc495ce7d34d6bb883a1095fce4c6b6cc1b82))
* **docker:** add dev stage with pre-installed Rust tooling ([0093d8b](https://github.com/michalschroeder/ci-tui/commit/0093d8b4bc34670762e501b0979b56ca223f5a40))
* **docker:** remove docker-compose from runtime image ([76ac35b](https://github.com/michalschroeder/ci-tui/commit/76ac35b8ef3adcf8aa28572fe4b28afc665e4ee9))
* **quick-005:** add time to build datetime display ([255aeec](https://github.com/michalschroeder/ci-tui/commit/255aeececf1204e713b295592b42c07b19153914))
* **quick-006:** add git refresh to r-hotkey before retry ([c02b7fe](https://github.com/michalschroeder/ci-tui/commit/c02b7fe800349e5b707aa09b67ea27d39402041a))
* **quick-007:** remove '+N more' text from Files section, use 'e' to expand ([a6a2498](https://github.com/michalschroeder/ci-tui/commit/a6a2498f432453b486aeaf5bc40b8c1c0e23fc87))
* **quick-009:** add container name support to DockerConfig ([72e3a88](https://github.com/michalschroeder/ci-tui/commit/72e3a88211b0e82090af96e3c6311b9ed80d7032))
* **quick-009:** replace docker compose exec with docker exec/run ([9b73d4c](https://github.com/michalschroeder/ci-tui/commit/9b73d4cea0dcbfbb1173029fb5d8b46a5c795831))
* **quick-009:** update simple.rs to use docker exec/run ([a3859bf](https://github.com/michalschroeder/ci-tui/commit/a3859bf2ab852f8a4b57c343512669852816faa8))
* **quick-011:** create ci-tui.yaml self-hosting config ([e42634c](https://github.com/michalschroeder/ci-tui/commit/e42634c31aab04f1dc4f23c11dabce8b2272fcd5))
* **quick-011:** extend DockerConfig with standalone run fields ([78c32e2](https://github.com/michalschroeder/ci-tui/commit/78c32e2f389faf40e627c0134153e0ab2ca9db8e))
* **quick-011:** update build_docker_run_command to use DockerConfig fields ([295746e](https://github.com/michalschroeder/ci-tui/commit/295746e01e6994855c6dc0b1e3e45201adc10047))
* **quick-012:** add scroll position indicator to output panel titles ([8840214](https://github.com/michalschroeder/ci-tui/commit/88402143313c51eb83528575c9f03235f5e90696))
* **quick-012:** fix output scroll bounds with dynamic visible lines ([1ef7a34](https://github.com/michalschroeder/ci-tui/commit/1ef7a3402379f264e0ca0d213fa05ada70e6af00))
* **quick-013:** add deny_unknown_fields to all config structs ([f66485a](https://github.com/michalschroeder/ci-tui/commit/f66485aad5f20746f5466974d1e47d4911cbede2))
* **quick-014:** add CD workflow for Docker image publishing ([9589c5e](https://github.com/michalschroeder/ci-tui/commit/9589c5e4c24b8182c0a4db6d470fe6e18cac5227))
* **quick-014:** add Docker build verification to CI workflow ([a3be1b1](https://github.com/michalschroeder/ci-tui/commit/a3be1b105c597ccdc1b85aece1db5567d10ef67e))
* **quick-015:** add Release Please for automated releases ([80b6ec6](https://github.com/michalschroeder/ci-tui/commit/80b6ec665eb59a30a0c9c2ca970af5ec5dcd7d79))


### Bug Fixes

* **01-02:** add missing UI constants ([41c4f58](https://github.com/michalschroeder/ci-tui/commit/41c4f58fb56f596df44c319fe491a9db22a3157a))
* **01:** revise plans based on checker feedback ([768172a](https://github.com/michalschroeder/ci-tui/commit/768172aa6771e6b1b05191c0b870076ce4d34161))
* **build:** pass git hash and date as Docker build args ([8c5ebab](https://github.com/michalschroeder/ci-tui/commit/8c5ebabfeab685a29b7b9ecffc9d99caebcc6b09))
* **config:** resolve "." path to actual directory name ([907ba80](https://github.com/michalschroeder/ci-tui/commit/907ba80afaf4f1fda2d51576cd3ce0fcd959587c))
* **deps:** update rust crate crossterm to 0.29 ([67d0d34](https://github.com/michalschroeder/ci-tui/commit/67d0d341cb468facc8aea5f7484a14fad94a2dd8))
* **deps:** update rust crate crossterm to 0.29 ([88916fd](https://github.com/michalschroeder/ci-tui/commit/88916fd044c2cfd5c8f39f733fad0d5194e405db))
* **deps:** update rust crate sysinfo to 0.37 ([ec03306](https://github.com/michalschroeder/ci-tui/commit/ec033065617e7d9006d18bf465dbc8588ce59ef9))
* **deps:** update rust crate sysinfo to 0.37 ([a564532](https://github.com/michalschroeder/ci-tui/commit/a564532f1204efc174fc6ea5fb642cea361b0086))
* **dockerfile:** correct OCI image source label ([ed4a4f3](https://github.com/michalschroeder/ci-tui/commit/ed4a4f3951f5b796e21f52f0fde0b7fa6308d8b6))
* **makefile:** quote BUILD_DATE arg to handle spaces in timestamp ([6446638](https://github.com/michalschroeder/ci-tui/commit/6446638bf7cef5e4126718a0bb66e7bf0bcb2fc3))
* **makefile:** run cargo commands in Docker for local development ([3bae049](https://github.com/michalschroeder/ci-tui/commit/3bae049f6521fb08e8fc6cccc41ed7406feedda5))
* **quick-004:** filter pre-commands when StatusFilter::Failed is active ([171fa5c](https://github.com/michalschroeder/ci-tui/commit/171fa5c6c889007af5fa8b24ef06512e4f309f9e))


### Code Refactoring

* apply rustfmt formatting across codebase ([649e4c5](https://github.com/michalschroeder/ci-tui/commit/649e4c5cf57579354b0331167472e4d025ba7fdc))
* **config:** simplify checks and use custom dev container ([99093af](https://github.com/michalschroeder/ci-tui/commit/99093af3208e494298c0f287c0455f962c4cb9e3))
* **docker:** pass DockerConfig instead of container_name string ([62679a0](https://github.com/michalschroeder/ci-tui/commit/62679a0d0646393edb3edb5e376d5378089a06b0))
* **makefile:** extract DOCKER_BUILD variable to reduce duplication ([08b3513](https://github.com/michalschroeder/ci-tui/commit/08b3513b62e3be692425a7aa7929d89672ae99bd))
* **quick-001:** reorganize Makefile with consistent local cargo approach ([e406114](https://github.com/michalschroeder/ci-tui/commit/e406114d66772977ab136890d5e114b2988ef925))
* **test:** fix formatting in config test assertion ([3becdf1](https://github.com/michalschroeder/ci-tui/commit/3becdf18f08b1ba2a2cf5574b585a373cfee295e))
* use `&Path` instead of `&PathBuf` for function parameters ([59f0bc4](https://github.com/michalschroeder/ci-tui/commit/59f0bc40cb6b88aa7c18d68ebb7ed700bf2a7a66))


### Documentation

* **002:** complete version display quick task ([459ec3c](https://github.com/michalschroeder/ci-tui/commit/459ec3c9d90b6a76bec734e0fcfdafc9720ebc5f))
* **003:** add quick task plan ([07f8288](https://github.com/michalschroeder/ci-tui/commit/07f8288e2aca16e712482b8e5d23248ce1b09488))
* **003:** complete remove unused run.sh quick task ([2da3ab9](https://github.com/michalschroeder/ci-tui/commit/2da3ab922dec032e4131a9f6de7c6a29e4110747))
* **01-01:** complete Responsive Event Loop plan ([8586816](https://github.com/michalschroeder/ci-tui/commit/8586816219218e88a1db9719d7697bb4bc2aa9aa))
* **01-02:** complete Testing Infrastructure plan ([16e8ab0](https://github.com/michalschroeder/ci-tui/commit/16e8ab06f2ce8e8d19965a11bd83c048d77e2d55))
* **01-03:** complete CI Pipeline plan ([1d07fb3](https://github.com/michalschroeder/ci-tui/commit/1d07fb3c708c7a081d28a73d0040f8d6fe6f5343))
* **01:** complete Foundation phase ([b6a81da](https://github.com/michalschroeder/ci-tui/commit/b6a81da629a3768276b8532cbea7ab7467481351))
* **01:** create phase 1 plan ([065317b](https://github.com/michalschroeder/ci-tui/commit/065317b6195faa84477ef26d2be1dae216c2ecaf))
* **01:** research phase domain ([2fb2c01](https://github.com/michalschroeder/ci-tui/commit/2fb2c0150d59938a2ef6e1c12bfa44d682ac1da8))
* archive completed quick tasks 001-003 ([908f083](https://github.com/michalschroeder/ci-tui/commit/908f083643f34f9e3c1a51738de3a298f22c7b2b))
* complete project research ([c98ad6c](https://github.com/michalschroeder/ci-tui/commit/c98ad6c7ea6e5537442afc9ea88fffedb5d94140))
* create roadmap (5 phases) ([1c4a97a](https://github.com/michalschroeder/ci-tui/commit/1c4a97a9034986d6d5957889f63f9530a217a104))
* define v1 requirements ([331d7b7](https://github.com/michalschroeder/ci-tui/commit/331d7b7a887745c3cbbd4d9cb6d775c832df5d1f))
* initialize project ([913dc77](https://github.com/michalschroeder/ci-tui/commit/913dc7786a0fc66485fc59bbe9037b51db1c18f9))
* map existing codebase ([5b9fa1a](https://github.com/michalschroeder/ci-tui/commit/5b9fa1af603bae7b936761251fe988546bfd4a36))
* **quick-001:** archive plan for Docker image optimization ([d73bafc](https://github.com/michalschroeder/ci-tui/commit/d73bafc75b4b2660795a0028ba114329c76029f8))
* **quick-001:** complete Docker build optimization plan ([390c484](https://github.com/michalschroeder/ci-tui/commit/390c484e73b0a0798611fae813c806cc8fbafc1a))
* **quick-001:** complete Makefile refactor task ([b961357](https://github.com/michalschroeder/ci-tui/commit/b96135789ab25ef133d49bb985f2a4613f1ad014))
* **quick-001:** Refactor Makefile for consistent cargo approach ([30f9aa3](https://github.com/michalschroeder/ci-tui/commit/30f9aa333b874a31fdad21454e18d6c634639ca2))
* **quick-004:** add planning artifact ([da7cb58](https://github.com/michalschroeder/ci-tui/commit/da7cb58be924ad4374f7369e16c9fd736496cf80))
* **quick-004:** complete fix f-hotkey plan ([805f880](https://github.com/michalschroeder/ci-tui/commit/805f880ce867e5ee0f3574167268c467ed2ddf9f))
* **quick-005:** archive show full datetime in build footer plan ([3f0a63b](https://github.com/michalschroeder/ci-tui/commit/3f0a63b26300b048017117962a5f7d46aa373d13))
* **quick-005:** complete show full datetime in build footer plan ([61a2b6a](https://github.com/michalschroeder/ci-tui/commit/61a2b6a6e6563d6953d79a5379e3b9624ee8d0f2))
* **quick-006:** archive r-hotkey refresh git changes plan ([076beab](https://github.com/michalschroeder/ci-tui/commit/076beab2faf3cc22f326ebc70db4a9d38d067db5))
* **quick-006:** complete r-hotkey refresh git changes plan ([0fc5529](https://github.com/michalschroeder/ci-tui/commit/0fc5529c085d1aae79388fc7b958449dfcf6ef3e))
* **quick-007:** add planning artifact ([d552c87](https://github.com/michalschroeder/ci-tui/commit/d552c8707a234e194781432621afb7c00e9ba850))
* **quick-007:** complete remove '+N more' from files section plan ([94e7219](https://github.com/michalschroeder/ci-tui/commit/94e7219c8b5e19ef27e69998f4bc42ff4f27dd82))
* **quick-008:** complete research self-hosting requirement plan ([0b22be9](https://github.com/michalschroeder/ci-tui/commit/0b22be9a5f90586d6be3f7c9bbd3618a164e116f))
* **quick-008:** remove completed self-hosting research task ([dff0b6d](https://github.com/michalschroeder/ci-tui/commit/dff0b6d196b70a82b57ca34b0a914f140de709d5))
* **quick-008:** research self-hosting requirements for CI-TUI ([05aa45e](https://github.com/michalschroeder/ci-tui/commit/05aa45e57bd581d1d20873c3a35cfd2c1119e01e))
* **quick-009:** archive quick task plan artifact ([0927092](https://github.com/michalschroeder/ci-tui/commit/09270925c1e566676fb2306db7f4edd699f0fcd0))
* **quick-009:** complete replace docker-compose exec with docker exec/run plan ([a8be408](https://github.com/michalschroeder/ci-tui/commit/a8be408dc29cd66860504a2549654afac3acfa68))
* **quick-010:** remove docker-compose from Docker image ([77556cd](https://github.com/michalschroeder/ci-tui/commit/77556cd518faa49e5d94ff1fa3e4f50763d6ef6a))
* **quick-011:** archive quick task plan artifact ([4aee532](https://github.com/michalschroeder/ci-tui/commit/4aee532cb42369dffa3aa9900bd2d5070514a876))
* **quick-011:** complete create CI-TUI config for self-hosting plan ([a359c9f](https://github.com/michalschroeder/ci-tui/commit/a359c9f5cde86b16da0dca42640c2e8c437190c1))
* **quick-012:** archive quick task plan artifact ([c251fe6](https://github.com/michalschroeder/ci-tui/commit/c251fe671bec03921ceb1103df8da866b38bc845))
* **quick-012:** complete fix check output panel scrolling plan ([38a1886](https://github.com/michalschroeder/ci-tui/commit/38a188640b32046e68b54db66184030976f570fe))
* **quick-013:** add config validation with schema error messages ([4759f48](https://github.com/michalschroeder/ci-tui/commit/4759f489263aeb0de88d28653d8761ec07bfdb1c))
* **quick-013:** complete config validation quick task ([956c546](https://github.com/michalschroeder/ci-tui/commit/956c54644919dcde184df610edef0efb991b1ab5))
* **quick-014:** add CI/CD pipeline with GitHub Actions ([a054c1a](https://github.com/michalschroeder/ci-tui/commit/a054c1a6f2b19333d40bdb0b37e20698e5af2e67))
* **quick-014:** complete CI/CD pipeline quick task ([7f6c902](https://github.com/michalschroeder/ci-tui/commit/7f6c902e6b3a8a2bce6c963e76edee750d7030da))
* **quick-015:** add CONTRIBUTING.md with release process ([7a5530d](https://github.com/michalschroeder/ci-tui/commit/7a5530d0eb359e5fe1f66728e9db8245de427c7f))
* **quick-015:** add Release Please for automated releases ([820cada](https://github.com/michalschroeder/ci-tui/commit/820cada8afeb86367b9b97e23625a5ec2598cf7d))
* **quick-015:** complete Release Please quick task ([d58b510](https://github.com/michalschroeder/ci-tui/commit/d58b510fc6d984cc1ce1a5273ca690ee7d379b90))
* **quick:** archive completed planning artifacts for tasks 001-007 ([167e2e9](https://github.com/michalschroeder/ci-tui/commit/167e2e90dca844b73610d28e3a79c357346ccf62))
* **quick:** archive quick task 008 plan artifact ([979b111](https://github.com/michalschroeder/ci-tui/commit/979b11166ef340f417fa7613d4bd4c337d31656b))
