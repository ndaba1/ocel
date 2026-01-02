export class UploadError extends Error {
  public readonly statusCode: number;
  public readonly code: string;

  constructor(
    message: string,
    statusCode = 500,
    code = "INTERNAL_SERVER_ERROR"
  ) {
    super(message);
    this.statusCode = statusCode;
    this.code = code;

    // Essential for 'instanceof' to work correctly when extending built-in classes in TypeScript
    Object.setPrototypeOf(this, UploadError.prototype);
  }

  // --- Static Factory Helpers (The DX Magic) ---

  /**
   * 400 Bad Request
   * Use when the input is invalid (e.g. file too big, wrong extension)
   */
  static badRequest(message = "Bad Request") {
    return new UploadError(message, 400, "BAD_REQUEST");
  }

  /**
   * 401 Unauthorized
   * Use when the user is not logged in or missing headers
   */
  static unauthorized(message = "Unauthorized") {
    return new UploadError(message, 401, "UNAUTHORIZED");
  }

  /**
   * 403 Forbidden
   * Use when the user is logged in but doesn't have permission (e.g. wrong plan)
   */
  static forbidden(message = "Forbidden") {
    return new UploadError(message, 403, "FORBIDDEN");
  }

  /**
   * 404 Not Found
   * Use when a resource required for the upload is missing
   */
  static notFound(message = "Not Found") {
    return new UploadError(message, 404, "NOT_FOUND");
  }

  /**
   * 429 Too Many Requests
   * Use for rate limiting logic
   */
  static tooManyRequests(message = "Too many requests") {
    return new UploadError(message, 429, "TOO_MANY_REQUESTS");
  }

  /**
   * 500 Internal Server Error
   * Use for unexpected crashes or database failures
   */
  static internal(message = "Internal Server Error") {
    return new UploadError(message, 500, "INTERNAL_SERVER_ERROR");
  }
}
