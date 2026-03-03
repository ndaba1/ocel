import { lambda } from "ocel/lambda";

export const hello2Fn = lambda("helloFunction2", {
  handler: async () => {
    return {
      statusCode: 200,
      body: JSON.stringify({
        message: "Hello world",
      }),
    };
  },
  trigger: {
    type: "url",
    config: {},
  },
});
