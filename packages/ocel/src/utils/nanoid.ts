import { customAlphabet } from "nanoid";

export function getNanoid(length = 7) {
  const alphabet =
    "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
  const nanoid = customAlphabet(alphabet, length);

  return nanoid();
}
