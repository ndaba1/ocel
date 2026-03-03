import { lambda } from "ocel/lambda";

export default lambda("storageFunction", {
  async handler(event, context) {
    // return { msg: "Hello World!!!!" };

    return {
      statusCode: 200,
      headers: {
        "Content-Type": "application/json",
      },
      body: JSON.stringify({
        message: "Hello from Lambda",
      }),
    };
  },
  trigger: {
    type: "api",
    config: {
      method: "GET",
      path: "/hello",
    },

    // type: "url",
    // config: {},
  },
});
