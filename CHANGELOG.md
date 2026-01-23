# Changelog

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
