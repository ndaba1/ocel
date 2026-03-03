import { sleep } from "workflow";

export async function handleUserSignup(email: string) {
  "use workflow";
  const user = await createUser(email);
  await sendWelcomeEmail(user);
  await sleep("5s"); // Pause for 5s - doesn't consume any resources
  await sendOnboardingEmail(user);
  console.log(
    "Workflow is complete! Run 'npx workflow web' to inspect your run"
  );
  return { userId: user.id, status: "onboarded" };
}

async function createUser(email: string) {
  "use step";
  // Simulate user creation logic
  console.log(`Creating user with email: ${email}`);
  return { id: "user123", email };
}

async function sendWelcomeEmail(user: { id: string; email: string }) {
  "use step";
  // Simulate sending a welcome email
  console.log(`Sending welcome email to: ${user.email}`);
}

async function sendOnboardingEmail(user: { id: string; email: string }) {
  "use step";
  // Simulate sending an onboarding email
  console.log(`Sending onboarding email to: ${user.email}`);
}
