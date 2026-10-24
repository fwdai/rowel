# Contributing to Rowel

:tada: First off, thanks for taking the time to contribute! :tada:

This project adheres to the Contributor Covenant [code of conduct](CODE_OF_CONDUCT.md).
By participating, you are expected to uphold this code.

The following are guidelines for contributing to Rowel.
These are just guidelines, not rules, use your best judgment and feel free to
propose changes to this document in a pull request.

## [Issues](https://github.com/fwdai/rowel/issues)

Issues are created [here](https://github.com/fwdai/rowel/issues/new).

### Issue Maintenance and Closure
* If an issue is inactive for 45 days (no activity of any kind), it will be marked for closure with `stale`.
* If after this label is applied, no further activity occurs in the next 7 days, the issue will be closed.
* If an issue has been closed and you still feel it's relevant, feel free to ping a maintainer or add a comment!

## [Pull Requests](https://github.com/fwdai/rowel/pulls)

Pull Requests are the way concrete changes are made to the code, documentation,
dependencies, and tools contained in the `fwdai/rowel` repository.

* Setting up your local environment
  * Fork the project on GitHub and clone your fork locally.
  ```
  $ git clone git@github.com:fwdai/rowel.git
  $ cd rowel
  $ git remote add upstream https://github.com/fwdai/rowel.git
  $ git fetch upstream
  ```
  * Install dependencies with:
  `bun install`
  * Start app in development with:
  `bun run tauri:dev`
  * Run Tests with:
  `bun run test`
  * To keep your development environment organized, create local branches to hold your work. These should be branched directly off of the master branch.
  ```
  $ git checkout -b my-branch -t upstream/master
  ```
* Making Changes
  * Make a change to the code
  * Commit your changes using [semantic commit messages](https://www.conventionalcommits.org/en/v1.0.0-beta.4/) to streamline the release process. A good commit message should describe what changed and why.
  * Once you have committed your changes, it is a good idea to use git rebase
  ```
  $ git fetch upstream
  $ git rebase upstream/master
  ```
  * Before submitting your changes in a pull request, always run the full test suite. To run the tests:
  ```
  bun run test
  cd src-tauri && cargo test
  ```

  * If a change starts something in one call and finishes it in another (a browser hand-off, a staged file, a background run), treat the in-between as a value with a lifecycle, not a flag: give it an identity that the finishing side has to prove, a way to expire, and a teardown that runs on *every* exit, ideally a `Drop`. The backend owns it and announces each transition; the frontend mirrors, never guesses. List the states and every way out of each in the PR description, and give each exit a test — "happy path plus cancel" is where these bugs hide.
  * Once your commits are ready to go (with passing tests and linting) begin the process of opening a pull request by pushing your working branch to your fork on GitHub.
  * From within GitHub, opening a new pull request will present you with a template that should be filled out

## Licensing and sign-off

Two licenses live in this repository, by directory (see the License section of
the README):

* `src-tauri/crates/*` (the Rust library crates) are **MIT OR Apache-2.0**.
* Everything else (the desktop and iOS apps, the frontend, the browser
  extension, docs) is **GPL-3.0**.

A contribution is licensed under the license of the directory it lands in, with
no additional terms (inbound = outbound). To make that explicit and to certify
that you have the right to submit the change, every commit must carry a
[Developer Certificate of Origin](https://developercertificate.org/) sign-off:

```
git commit -s
```

which adds a `Signed-off-by: Your Name <you@example.com>` line. Pull requests
whose commits lack it will be asked to add it before review.

## Security

* **Never log secret-bearing values.** `log::*!` / `console.*` calls must not interpolate an `Entry`, its secret fields (password, notes, card, OTP), the master password, or any derived key. Log opaque errors and identifiers only.
  * Discuss changes and update pull request
