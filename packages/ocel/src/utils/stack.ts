/**
 * Returns absolute path of the caller file.
 */
export function getCallerFile(depth = 1): string | null {
  const original = Error.prepareStackTrace;

  try {
    Error.prepareStackTrace = (_, stack) => stack;
    const err = new Error();

    const frames = err.stack as unknown as NodeJS.CallSite[];

    // index 0 = getCallerFile
    // index 1 = the function that called getCallerFile
    return (
      frames[2 + depth]
        ?.getFileName()
        ?.replace("file://", "")
        ?.split("?")?.[0] ?? null
    );
  } catch {
    return null;
  } finally {
    Error.prepareStackTrace = original;
  }
}
