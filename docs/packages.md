# Packages, namespaces, and the manager that comes last

*Status: both halves are implemented — visibility and namespaces (see
[the reference](language.md#13-modules-and-visibility)). What follows is the
design as built, and the argument for why there is no package manager yet.*

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

**Not yet, and the reason is not effort.** A package manager is a promise
about *other people's code*: that a name means one thing, that a version
means what it says, that what was fetched yesterday is what is fetched
today. Keal cannot honestly make any of the three right now.

* **A name means one thing** — only once namespaces exist. Publishing to a
  registry before that would mint the exact collisions the language cannot
  yet express a way out of.
* **A version means what it says** — only once the semantics are frozen
  enough that a minor release cannot change what a correct program does.
  `RELEASING.md` says plainly that they are not: the cycle audit, typed
  exceptions and now namespaces are all still open.
* **Reproducibility** — a lockfile is easy; a registry that never rewrites
  history is a service somebody has to run, and a compiler project should
  not be running one before it has users.

### What to do instead, in order

1. **Namespaces first.** Nothing about dependencies can be designed
   honestly before a program can say which `parse` it means.
2. **Then a manifest, and only that.** A `keal.toml` naming the project,
   its version, and its dependencies as *git URLs with a commit or tag* —
   no registry, no resolution, no network protocol of its own. `keal fetch`
   clones into `.keal/deps/`, `import "dep:geometry/shapes.keal"` reads
   from there. Git already provides the naming, the versioning and the
   immutability; borrowing them costs nothing and owes nobody a service.
3. **A lockfile when there is transitivity.** The moment a dependency has
   dependencies, record the exact commits. Not before.
4. **A registry last, if ever.** It is worth building when there are enough
   packages that finding one is the problem. Until then it is infrastructure
   in search of a user, and `cargo install --git` is proof the middle step
   is enough for a long time.

The honest summary: a package manager is a distribution problem, and this
project's remaining problems are still language problems. When `import`
finally says exactly which code it means, the distribution question becomes
easy — and until then, no amount of tooling would make it right.
