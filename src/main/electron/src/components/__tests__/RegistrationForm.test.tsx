import React from 'react'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { vi, describe, it, expect, beforeEach } from 'vitest'
import RegistrationForm from '../RegistrationForm'
import { authService } from '../../services/authService'

// Mock the auth service
vi.mock('../../services/authService', () => ({
  authService: {
    register: vi.fn(),
  }
}))

// Mock framer-motion to avoid animation issues in tests
vi.mock('framer-motion', () => ({
  motion: {
    div: ({ children, ...props }: any) => <div {...props}>{children}</div>,
  },
  AnimatePresence: ({ children }: any) => <>{children}</>,
}))

describe('RegistrationForm', () => {
  const mockOnRegistrationSuccess = vi.fn()
  const mockOnSwitchToLogin = vi.fn()

  beforeEach(() => {
    vi.clearAllMocks()
  })

  const renderRegistrationForm = () => {
    return render(
      <RegistrationForm
        onRegistrationSuccess={mockOnRegistrationSuccess}
        onSwitchToLogin={mockOnSwitchToLogin}
      />
    )
  }

  it('renders registration form with all required fields', () => {
    renderRegistrationForm()

    expect(screen.getByRole('heading', { name: 'Create Account' })).toBeInTheDocument()
    expect(screen.getByText('Join EFS to secure your files with encryption')).toBeInTheDocument()
    expect(screen.getByPlaceholderText('Choose a username')).toBeInTheDocument()
    expect(screen.getByPlaceholderText('Create a strong password')).toBeInTheDocument()
    expect(screen.getByPlaceholderText('Confirm your password')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /create account/i })).toBeInTheDocument()
    expect(screen.getByText('Sign in instead')).toBeInTheDocument()
  })

  it('validates required fields', async () => {
    renderRegistrationForm()

    const submitButton = screen.getByRole('button', { name: /create account/i })
    fireEvent.click(submitButton)

    await waitFor(() => {
      expect(screen.getByText('Please enter a username')).toBeInTheDocument()
      expect(screen.getByText('Please enter a password')).toBeInTheDocument()
      expect(screen.getByText('Please confirm your password')).toBeInTheDocument()
    })
  })

  it('validates username format and length', async () => {
    renderRegistrationForm()

    const usernameInput = screen.getByPlaceholderText('Choose a username')
    const submitButton = screen.getByRole('button', { name: /create account/i })

    // Test minimum length
    fireEvent.change(usernameInput, { target: { value: 'ab' } })
    fireEvent.click(submitButton)

    await waitFor(() => {
      expect(screen.getByText('Username must be at least 3 characters')).toBeInTheDocument()
    })

    // Test invalid characters
    fireEvent.change(usernameInput, { target: { value: 'user@name' } })
    fireEvent.click(submitButton)

    await waitFor(() => {
      expect(screen.getByText('Username can only contain letters, numbers, underscores, and hyphens')).toBeInTheDocument()
    })

    // Test maximum length
    fireEvent.change(usernameInput, { target: { value: 'a'.repeat(25) } })
    fireEvent.click(submitButton)

    await waitFor(() => {
      expect(screen.getByText('Username must be less than 20 characters')).toBeInTheDocument()
    })
  })

  it('validates password strength', async () => {
    renderRegistrationForm()

    const passwordInput = screen.getByPlaceholderText('Create a strong password')
    
    // Test weak password
    fireEvent.change(passwordInput, { target: { value: 'weak' } })
    
    await waitFor(() => {
      expect(screen.getByText('At least 8 characters')).toBeInTheDocument()
      expect(screen.getByText('One uppercase letter')).toBeInTheDocument()
      expect(screen.getByText('One number')).toBeInTheDocument()
      expect(screen.getByText('One special character (@$!%*?&)')).toBeInTheDocument()
    })

    // Test strong password
    fireEvent.change(passwordInput, { target: { value: 'StrongPass123!' } })
    
    await waitFor(() => {
      expect(screen.getByText('Strong password!')).toBeInTheDocument()
    })
  })

  it('validates password confirmation', async () => {
    renderRegistrationForm()

    const passwordInput = screen.getByPlaceholderText('Create a strong password')
    const confirmPasswordInput = screen.getByPlaceholderText('Confirm your password')
    const submitButton = screen.getByRole('button', { name: /create account/i })

    fireEvent.change(passwordInput, { target: { value: 'StrongPass123!' } })
    fireEvent.change(confirmPasswordInput, { target: { value: 'DifferentPass123!' } })
    fireEvent.click(submitButton)

    await waitFor(() => {
      expect(screen.getByText('Passwords do not match')).toBeInTheDocument()
    })
  })

  it('shows password strength indicator with progress bar', async () => {
    renderRegistrationForm()

    const passwordInput = screen.getByPlaceholderText('Create a strong password')
    
    // Enter a password to trigger strength indicator
    fireEvent.change(passwordInput, { target: { value: 'Test123!' } })
    
    await waitFor(() => {
      // Should show progress bar (Ant Design Progress component)
      const progressBar = document.querySelector('.ant-progress')
      expect(progressBar).toBeInTheDocument()
    })
  })

  it('calls authService.register with correct data on form submission', async () => {
    const mockRegistrationResult = {
      success: true,
      user: { id: '1', username: 'testuser' },
      token: 'mock-token'
    }
    
    vi.mocked(authService.register).mockResolvedValue(mockRegistrationResult)
    
    renderRegistrationForm()

    const usernameInput = screen.getByPlaceholderText('Choose a username')
    const passwordInput = screen.getByPlaceholderText('Create a strong password')
    const confirmPasswordInput = screen.getByPlaceholderText('Confirm your password')
    const submitButton = screen.getByRole('button', { name: /create account/i })

    fireEvent.change(usernameInput, { target: { value: 'testuser' } })
    fireEvent.change(passwordInput, { target: { value: 'StrongPass123!' } })
    fireEvent.change(confirmPasswordInput, { target: { value: 'StrongPass123!' } })
    fireEvent.click(submitButton)

    await waitFor(() => {
      expect(authService.register).toHaveBeenCalledWith({
        username: 'testuser',
        password: 'StrongPass123!',
        confirmPassword: 'StrongPass123!'
      })
    })
  })

  it('calls onRegistrationSuccess when registration is successful', async () => {
    const mockUser = { id: '1', username: 'testuser' }
    const mockRegistrationResult = {
      success: true,
      user: mockUser,
      token: 'mock-token'
    }
    
    vi.mocked(authService.register).mockResolvedValue(mockRegistrationResult)
    
    renderRegistrationForm()

    const usernameInput = screen.getByPlaceholderText('Choose a username')
    const passwordInput = screen.getByPlaceholderText('Create a strong password')
    const confirmPasswordInput = screen.getByPlaceholderText('Confirm your password')
    const submitButton = screen.getByRole('button', { name: /create account/i })

    fireEvent.change(usernameInput, { target: { value: 'testuser' } })
    fireEvent.change(passwordInput, { target: { value: 'StrongPass123!' } })
    fireEvent.change(confirmPasswordInput, { target: { value: 'StrongPass123!' } })
    fireEvent.click(submitButton)

    await waitFor(() => {
      expect(mockOnRegistrationSuccess).toHaveBeenCalledWith(mockUser)
    })
  })

  it('displays error message when registration fails', async () => {
    const mockRegistrationResult = {
      success: false,
      error: 'Username already exists'
    }
    
    vi.mocked(authService.register).mockResolvedValue(mockRegistrationResult)
    
    renderRegistrationForm()

    const usernameInput = screen.getByPlaceholderText('Choose a username')
    const passwordInput = screen.getByPlaceholderText('Create a strong password')
    const confirmPasswordInput = screen.getByPlaceholderText('Confirm your password')
    const submitButton = screen.getByRole('button', { name: /create account/i })

    fireEvent.change(usernameInput, { target: { value: 'existinguser' } })
    fireEvent.change(passwordInput, { target: { value: 'StrongPass123!' } })
    fireEvent.change(confirmPasswordInput, { target: { value: 'StrongPass123!' } })
    fireEvent.click(submitButton)

    await waitFor(() => {
      expect(screen.getByText('Username already exists')).toBeInTheDocument()
    })
  })

  it('shows loading state during registration', async () => {
    // Create a promise that we can control
    let resolveRegistration: (value: any) => void
    const registrationPromise = new Promise((resolve) => {
      resolveRegistration = resolve
    })
    
    vi.mocked(authService.register).mockReturnValue(registrationPromise)
    
    renderRegistrationForm()

    const usernameInput = screen.getByPlaceholderText('Choose a username')
    const passwordInput = screen.getByPlaceholderText('Create a strong password')
    const confirmPasswordInput = screen.getByPlaceholderText('Confirm your password')
    const submitButton = screen.getByRole('button', { name: /create account/i })

    fireEvent.change(usernameInput, { target: { value: 'testuser' } })
    fireEvent.change(passwordInput, { target: { value: 'StrongPass123!' } })
    fireEvent.change(confirmPasswordInput, { target: { value: 'StrongPass123!' } })
    fireEvent.click(submitButton)

    // Check loading state
    await waitFor(() => {
      expect(screen.getByText('Creating Account...')).toBeInTheDocument()
      expect(submitButton).toHaveClass('ant-btn-loading')
    })

    // Resolve the promise
    resolveRegistration!({ success: true, user: { id: '1', username: 'testuser' } })

    await waitFor(() => {
      expect(screen.queryByText('Creating Account...')).not.toBeInTheDocument()
    })
  })

  it('calls onSwitchToLogin when sign in link is clicked', () => {
    renderRegistrationForm()

    const signInLink = screen.getByText('Sign in instead')
    fireEvent.click(signInLink)

    expect(mockOnSwitchToLogin).toHaveBeenCalled()
  })

  it('clears error when form values change', async () => {
    const mockRegistrationResult = {
      success: false,
      error: 'Username already exists'
    }
    
    vi.mocked(authService.register).mockResolvedValue(mockRegistrationResult)
    
    renderRegistrationForm()

    const usernameInput = screen.getByPlaceholderText('Choose a username')
    const passwordInput = screen.getByPlaceholderText('Create a strong password')
    const confirmPasswordInput = screen.getByPlaceholderText('Confirm your password')
    const submitButton = screen.getByRole('button', { name: /create account/i })

    // First, trigger an error
    fireEvent.change(usernameInput, { target: { value: 'existinguser' } })
    fireEvent.change(passwordInput, { target: { value: 'StrongPass123!' } })
    fireEvent.change(confirmPasswordInput, { target: { value: 'StrongPass123!' } })
    fireEvent.click(submitButton)

    await waitFor(() => {
      expect(screen.getByText('Username already exists')).toBeInTheDocument()
    })

    // Then change form values to clear error
    fireEvent.change(usernameInput, { target: { value: 'newuser' } })

    await waitFor(() => {
      expect(screen.queryByText('Username already exists')).not.toBeInTheDocument()
    })
  })

  it('handles unexpected errors gracefully', async () => {
    vi.mocked(authService.register).mockRejectedValue(new Error('Network error'))
    
    renderRegistrationForm()

    const usernameInput = screen.getByPlaceholderText('Choose a username')
    const passwordInput = screen.getByPlaceholderText('Create a strong password')
    const confirmPasswordInput = screen.getByPlaceholderText('Confirm your password')
    const submitButton = screen.getByRole('button', { name: /create account/i })

    fireEvent.change(usernameInput, { target: { value: 'testuser' } })
    fireEvent.change(passwordInput, { target: { value: 'StrongPass123!' } })
    fireEvent.change(confirmPasswordInput, { target: { value: 'StrongPass123!' } })
    fireEvent.click(submitButton)

    await waitFor(() => {
      expect(screen.getByText('An unexpected error occurred. Please try again.')).toBeInTheDocument()
    })
  })



  it('prevents form submission with weak password', async () => {
    renderRegistrationForm()

    const usernameInput = screen.getByPlaceholderText('Choose a username')
    const passwordInput = screen.getByPlaceholderText('Create a strong password')
    const confirmPasswordInput = screen.getByPlaceholderText('Confirm your password')
    const submitButton = screen.getByRole('button', { name: /create account/i })

    fireEvent.change(usernameInput, { target: { value: 'testuser' } })
    fireEvent.change(passwordInput, { target: { value: 'weak' } })
    fireEvent.change(confirmPasswordInput, { target: { value: 'weak' } })
    fireEvent.click(submitButton)

    await waitFor(() => {
      expect(screen.getByText('Password is too weak')).toBeInTheDocument()
      expect(authService.register).not.toHaveBeenCalled()
    })
  })
})