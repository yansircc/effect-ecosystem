import { useQuery } from "@tanstack/react-query"
export const useUser = () => useQuery({ queryKey: ["user"], queryFn: () => 1 })
