# Packages, namespaces, and the index that is not a registry

*Status: all four steps are implemented — visibility and namespaces (see
[the reference](language.md#13-modules-and-visibility)), a manifest, a
lockfile, and an index for finding a package whose URL you do not know.
What follows is the design as built, and the argument that shaped it: at
every step, the smallest thing that does not oblige anybody to run a
service.*

## What is true today

`import "./geometry.keal"` reads a file, and everything that file lets out —
`public`, or `package` from a file in the same directory — becomes visible
under its own bare name. `import "./text.keal" as text` keeps those names
out of the bare set and reaches them through `text.` instead. Two files may
declare `parse`; the file that imports both says which one it means.

## What a namespace has to do here

Three requirements, in the order they matter:

1. **Two files may declare the same name.** The importing file says which
   one it means.
2. **A bare name keeps working.** The overwhelming case is one import, one
   `parse`, and no ambiguity; qualification must be the answer to a real
   collision, not a tax on every call.
3. **It must survive monomorphisation and C.** Generated C has one flat
   namespace of its own, so two `parse` functions need two symbols, and the
   emitter must agree with itself across the twin, byte for byte.

### The shape, as built

```keal
import "./geometry.keal"              // as today: bare names
import "./text.keal" as text          // qualified: text.parse(...)
```

* A file's **own** declarations come first: nothing an import brings in can
  shadow what the file itself declares.
* Then the **unqualified** imports, in one set. A name that lands in that set
  twice is not an error at import time — it is an error only where it is
  *used* unqualified, and the message names both files and says to alias one.
  Importing two libraries that happen to share a name must not break a
  program that never mentions it.
* An **aliased** import contributes nothing to the bare set. `text.parse` is
  the only way to reach it, which is what makes the alias a real answer.
* The alias is a name in the file that declares it, like any other, and it
  is not a value: `text` alone is refused by name.

### How it works inside

Before anything is checked, one pass walks every top-level declaration and
gives it a **unique name**: the source name for the first to claim it, and
`parse#2`, `parse#3` and so on for the others. `#` cannot be written in
Keal, so a minted name can never be one a program chose; the C backend
flattens it to `_dup2`, and the pass refuses a spelling whose flattened form
any file declares. Where nothing collides — which is every program written
so far — the unique name *is* the source name and nothing downstream can
tell the pass ran.

The same pass records what each file can see: itself, then everything its
unaliased imports reach, and the prelude, which is loaded rather than
imported. A written name is then resolved against that list, and the node in
the tree is rewritten to the unique name, so the interpreters, the VM and
the C emitter all name the declaration the checker chose. An alias
contributes nothing to that list: `text.parse` and `text.Node` are rewritten
to the unique name directly, which is what makes an alias a real answer to a
collision rather than a second chance at one.

Two candidates for one written name is an error **where the name is
written**, naming both files. Neither import is at fault, and a program that
never mentions the shared name never hears about it.

## Then: a package manager?

**Three of its four promises are kept, and the fourth is refused on
purpose.** A package manager is a promise
about *other people's code*: that a name means one thing, that a version
means what it says, that what was fetched yesterday is what is fetched
today. Only the last of the three is cheap to keep.

* **A name means one thing** — this one is now true, and it is what
  namespaces bought. Two dependencies may declare `parse`, and the program
  that uses both says which it means.
* **A version means what it says** — not yet, and this is the honest
  blocker. `RELEASING.md` says the semantics are not frozen: while a minor
  release can still change what a correct program does, a version range
  would be a lie, which is why a manifest names a **commit or a tag** and
  nothing looser.
* **Reproducibility** — a commit is reproducible by construction. A
  registry that never rewrites history is a service somebody has to run,
  and a compiler project should not be running one before it has users.
  That argument still stands, and the index described in step 4 is not
  that: it hosts no code, serves no requests, and can disappear without
  breaking a single build.

### What to do instead, in order

1. **Namespaces first.** ✅ Done: a program can say which `parse` it means.
2. **Then a manifest, and only that.** ✅ Done — this is what exists today:

   ```toml
   # keal.toml
   [package]
   name = "myproject"
   version = "0.1.0"

   [dependencies]
   geometry = { git = "https://github.com/someone/geometry", tag = "v1.2.0" }
   text     = { git = "https://github.com/other/text", rev = "9f2c1ab" }
   ```

   `keal fetch` clones each one into `.keal/deps/<name>/` and checks out the
   tag or commit named — nothing else, and nothing implicit. A program then
   writes `import "dep:geometry/shapes.keal"`, which reads
   `.keal/deps/geometry/shapes.keal` beside the nearest `keal.toml`.

   Two things follow from that shape, and both are deliberate. **Only
   `keal fetch` touches the network**: the compiler reads what is on disk,
   so a project that commits its `.keal/deps/` builds with no git at all,
   and the self-hosted twin — which must never differ from the oracle —
   needs no notion of fetching. And **git provides the naming, the
   versioning and the immutability**; borrowing them costs nothing and owes
   nobody a service to keep running.

3. **A lockfile when there is transitivity.** ✅ Done, and it arrived with
   the transitivity. A dependency's own `keal.toml` is read now, and what it
   asks for is fetched into the **same** `.keal/deps` — flat, not nested,
   because two copies of a library are two different sets of types and a
   program holding both could not say which it meant.

   Flat means two askers can disagree, and nothing here can reconcile them:
   a manifest names a commit, not a range, so there is no newer to pick.
   `keal fetch` says so and stops, naming both:

   ```
   error: two versions of `geometry` are wanted, and only one can be here
     = note: myproject wants tag v1.2.0 of https://github.com/someone/geometry
     = note: shapes wants tag v1.1.0 of https://github.com/someone/geometry
     = note: commits are pinned, so nothing can pick between them: change one manifest
   ```

   That is a worse error message than a resolver would give and a more
   honest one: the alternative is a tool choosing a version on the
   program's behalf under rules nobody wrote down.

   `keal.lock` records what each name actually resolved to — the commit, not
   the tag — and who asked for it. A tag can be moved; a commit cannot, so a
   checkout carrying the lockfile builds against what was read on the day it
   was read. Commit it.

   One thing follows from flatness and is worth stating: a `dep:` import
   resolves against the **outermost** `keal.toml` above the file, not the
   nearest. A library's own `dep:` imports therefore reach the project's
   copy of its dependency rather than looking inside the library, which is
   the only way one `.keal/deps` can serve everybody.
4. **An index, which is not a registry.** ✅ Done — and the distinction is
   the whole design. What was missing was never hosting or versioning; it
   was a way to **find a package whose URL you do not already know**. That
   is an address book, and an address book does not need a service.

   The index is an ordinary git repository holding one small file per
   package:

   ```toml
   # packages/geometry.toml
   [package]
   name = "geometry"
   git = "https://github.com/someone/geometry"
   description = "points, lines and the arithmetic between them"
   ```

   One file per package, so two people adding two packages never touch the
   same line, and contributing is a pull request that adds a file — reviewed
   by people, in public, with a history nobody can rewrite quietly. Git
   provides the hosting, the review and the immutability; borrowing them is
   the same trick `keal fetch` already turns, and it owes nobody a service
   to keep running.

   ```sh
   keal search arithmetic     # find it
   keal add geometry          # write it into keal.toml, pinned exactly
   keal fetch                 # put it where the import expects it
   ```

   `keal add` reads the URL from the index, asks *that repository* what tags
   it has, and writes one exact pin. `keal add geometry@v1.2.0` names the
   tag; without one, the newest **version** tag is taken — digits and dots
   with an optional leading `v`, compared number by number, so `v1.10.0`
   beats `v1.2.0` and `nightly` is not considered at all. A tag you type
   that the repository does not have is refused where you typed it, with the
   real ones listed.

   Three properties are worth stating, because they are what make this an
   index and not a registry:

   * **The index says where a package lives, and nothing else.** No
     versions, no ranges, no resolution. Versions stay the package's own git
     tags, where they already were.
   * **The choice happens once, at your keyboard.** `keal add` picking the
     newest tag is a convenience; a resolver deciding again on every build,
     under rules nobody read, is the thing this project refused. What was
     picked is printed and written into the manifest, and no later build
     repeats the decision.
   * **Nothing depends on the index existing.** A manifest names the
     package's own repository, never the index. If the index repository
     vanished tomorrow, every `keal.toml` in the world would still build —
     and a package that is not in the index works exactly as well, by naming
     its git URL directly. The index is a convenience for *finding*, and
     losing a convenience is not losing a build.

   The local copy lives beside the user, in `~/.keal/index`, and is
   **cloned when it is missing and never refreshed on its own**: two runs of
   `keal add` on the same day must not be able to write different things.
   `keal index update` is the command that changes what you are reading, and
   you ask for it. `KEAL_INDEX` points at a different one — a fork, a
   company's own list, a directory on disk.

   `keal index entry` prints the file this project would contribute, so
   publishing is copy, paste, pull request.

The honest summary: a package manager is a distribution problem, and this
project's remaining problems are still language problems. What the index
answers is the one distribution question that was actually in the way —
*where does this live?* — and it answers it without becoming something
somebody has to keep alive.
