import React from "react";
import { Box, Text } from "ink";
import TextInput from "ink-text-input";

type Props = {
  value: string;
  onChange: (val: string) => void;
  onSubmit: (val: string) => void;
  disabled?: boolean;
};

export function InputPrompt({ value, onChange, onSubmit, disabled }: Props) {
  if (disabled) {
    return (
      <Box>
        <Text dimColor>{"❯ thinking..."}</Text>
      </Box>
    );
  }

  return (
    <Box>
      <Text bold color="cyan">{"❯ "}</Text>
      <TextInput value={value} onChange={onChange} onSubmit={onSubmit} />
    </Box>
  );
}
