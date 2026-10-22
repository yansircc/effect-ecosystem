// The form libraries are not installed in the fixture toolchain; the point of the fixture is that importing them is not a finding.
declare module "@tanstack/react-form" { export const useForm: () => unknown }
declare module "react-hook-form" { export const useForm: () => unknown }
declare module "formik" { export const useFormik: () => unknown }
