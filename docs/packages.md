# Packages, namespaces, and the manager that comes last

*Status: the visibility half is implemented (see
[the reference](language.md#13-modules-and-visibility)). The namespace half
is designed here and not yet built. A package manager is argued against, for
now, at the end.*

## What is true today

`import "./geometry.keal"` reads a file, and everything that file lets out —
`public`, or `package` from a file in the same directory — becomes visible
under its own bare name. There is one namespace and no way to write a
qualified one. Two consequences follow, and only the second is a problem:

* A helper can be kept private. That is settled: a declaration says who may
  name it, and a package is a directory.
* **Two files cannot both declare `parse`.** If a program imports both, the
  checker reports the second as declared twice, and there is nothing the
  program can write to mean one rather than the other. This is what a
  namespace is for, and Keal does not have one yet.

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

### The shape

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

### What it costs inside

The checker's global scope stops being one map and becomes one per file,
with a resolution order (own → unqualified imports → builtins) and an
ambiguity error where two candidates survive. Classes and traits need the
same treatment, and a class's identity has to stop being its source name:
two `Node` types must be two types. The plan is to give every declaration a
**unique internal name** at collection — the source name where it is
unambiguous, a suffixed one where it is not — and to keep the source name
beside it for diagnostics. Everything downstream (the interpreters, the VM,
the C emitter, `keal layout`, `keal doc`) then keeps working on unique names
as it does today, and the four dumps change on both sides together.

That is the whole design. It is a day's work in the oracle and the same
again in the twin, and it is the last thing standing between this language
and a first release that will not have to break programs later.

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
