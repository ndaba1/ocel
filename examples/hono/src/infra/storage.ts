import { bucket, uploader } from "ocel/blob";

const uploaders = {
  avatars: uploader(
    {
      middleware: async () => {
        // TODO: do authentication checks here
      },
    },
    {
      accept: ["image/*"],
      path: { prefix: "avatars/", randomSuffix: true },
      onUploadComplete: async (data) => {
        console.log("Avatar upload complete:", data);
      },
    },
  ),
};

export const storageBucket = bucket("storageBucket", { uploaders });
