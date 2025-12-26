# Contributing to tgcryptfs

Thank you for your interest in contributing to tgcryptfs! This document provides guidelines and instructions for contributing to the project.

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Getting Started](#getting-started)
- [Development Setup](#development-setup)
- [Making Changes](#making-changes)
- [Testing](#testing)
- [Submitting Changes](#submitting-changes)
- [Style Guide](#style-guide)
- [Project Structure](#project-structure)

## Code of Conduct

By participating in this project, you agree to maintain a respectful and inclusive environment for all contributors. Please be kind, considerate, and constructive in all interactions.

## Getting Started

1. Fork the repository on GitHub
2. Clone your fork locally:
   ```bash
   git clone https://github.com/YOUR_USERNAME/tgcryptfs.git
   cd tgcryptfs
   ```
3. Add the upstream repository:
   ```bash
   git remote add upstream https://github.com/damienheiser/tgcryptfs.git
   ```

## Development Setup

### Prerequisites

- **Rust**: Install the latest stable Rust toolchain from [rustup.rs](https://rustup.rs/)
- **FUSE**: Install FUSE for your operating system
  - **Linux**: `sudo apt-get install libfuse-dev fuse` (Ubuntu/Debian)
  - **macOS**: `brew install macfuse`
- **Telegram API Credentials**: Get your API ID and Hash from [my.telegram.org/apps](https://my.telegram.org/apps)

### Building

```bash
# Build the project
cargo build

# Build with optimizations
cargo build --release
```

### Configuration

1. Copy the example environment file:
   ```bash
   cp .env.example .env
   ```
2. Edit `.env` and add your Telegram API credentials
3. Never commit your `.env` file with real credentials

## Making Changes

1. Create a new branch for your changes:
   ```bash
   git checkout -b feature/your-feature-name
   ```
2. Make your changes, following the [Style Guide](#style-guide)
3. Write or update tests as needed
4. Ensure all tests pass:
   ```bash
   cargo test
   ```
5. Format your code:
   ```bash
   cargo fmt
   ```
6. Check for common mistakes:
   ```bash
   cargo clippy -- -D warnings
   ```

## Testing

### Running Tests

```bash
# Run all tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run specific test
cargo test test_name

# Run doc tests
cargo test --doc
```

### Writing Tests

- Write unit tests in the same file as the code being tested
- Place integration tests in the `tests/` directory
- Use descriptive test names that explain what is being tested
- Test both success and failure cases
- Mock external dependencies (Telegram API) when possible

Example:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encryption_roundtrip() {
        // Test implementation
    }
}
```

## Submitting Changes

1. Commit your changes with clear, descriptive commit messages:
   ```bash
   git commit -m "feat: add support for file compression"
   ```

   Use conventional commit prefixes:
   - `feat:` - New features
   - `fix:` - Bug fixes
   - `docs:` - Documentation changes
   - `test:` - Test additions or changes
   - `refactor:` - Code refactoring
   - `perf:` - Performance improvements
   - `chore:` - Maintenance tasks

2. Push to your fork:
   ```bash
   git push origin feature/your-feature-name
   ```

3. Create a Pull Request:
   - Go to the [tgcryptfs repository](https://github.com/damienheiser/tgcryptfs)
   - Click "New Pull Request"
   - Select your fork and branch
   - Fill in the PR template with:
     - Description of changes
     - Motivation and context
     - Testing performed
     - Related issues (if any)

4. Address review feedback:
   - Make requested changes in your branch
   - Push updates to the same branch
   - The PR will automatically update

## Style Guide

### Rust Code Style

- Follow the official [Rust Style Guide](https://doc.rust-lang.org/nightly/style-guide/)
- Use `cargo fmt` to format code automatically
- Use `cargo clippy` to catch common mistakes
- Write clear comments for complex logic
- Use meaningful variable and function names
- Keep functions small and focused on a single task

### Code Organization

- Keep related functionality together in modules
- Use private visibility by default, expose only necessary APIs
- Document public APIs with doc comments (`///`)
- Include examples in doc comments when helpful

### Error Handling

- Use `Result<T, E>` for recoverable errors
- Use `thiserror` for defining custom error types
- Provide context when propagating errors
- Use `anyhow` for application-level error handling

### Security

- Never commit API keys, passwords, or secrets
- Always use the `.env` file for sensitive configuration
- Validate all user input
- Use constant-time comparisons for cryptographic operations
- Follow secure coding practices for cryptography

## Project Structure

```
tgcryptfs/
├── src/
│   ├── main.rs           # Entry point and CLI
│   ├── lib.rs            # Library root
│   ├── crypto/           # Encryption and key derivation
│   ├── storage/          # Local database and cache
│   ├── telegram/         # Telegram API client
│   └── filesystem/       # FUSE implementation
├── tests/                # Integration tests
├── .github/
│   └── workflows/        # CI/CD workflows
├── Cargo.toml           # Project dependencies
└── README.md            # Project documentation
```

## Questions or Need Help?

- Open an issue for bugs or feature requests
- Check existing issues before creating a new one
- Provide as much context as possible when reporting bugs:
  - Operating system and version
  - Rust version (`rustc --version`)
  - Steps to reproduce
  - Expected vs actual behavior
  - Relevant logs or error messages

## License

By contributing to tgcryptfs, you agree that your contributions will be licensed under the MIT License.

---

Thank you for contributing to tgcryptfs!
