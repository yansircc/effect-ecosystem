export const load = async () => {
  const one = await fetch("/one")
      const two = await fetch("/two")
  return [one, two]
}
