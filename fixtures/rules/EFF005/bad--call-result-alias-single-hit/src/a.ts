export const settle = (xs: Promise<number>[]) => {
  const all = Promise.all(xs)
  return all.then((values) => values.length)
}
