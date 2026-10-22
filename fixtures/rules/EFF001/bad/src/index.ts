export const parse = (text: string) => {
  try {
    return JSON.parse(text) as unknown
  } catch {
    return undefined
  }
}
