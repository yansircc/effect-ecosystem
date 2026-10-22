// Compile scaffolding: the runtimes whose `env` the snippet reads. The scanner ignores declaration files.
interface ImportMeta { readonly env: Record<string, string | undefined> }
declare const Bun: { readonly env: Record<string, string | undefined> }
