// vite-plus is not installed in the fixture toolchain; the manifest names its module, so declare the surface the fixture uses.
declare module "vite-plus/test" {
  export const it: (name: string | (() => unknown), run?: string | (() => unknown)) => void
  export const test: typeof it
  export const beforeAll: (run: () => unknown) => void
  export const afterAll: (run: () => unknown) => void
}
