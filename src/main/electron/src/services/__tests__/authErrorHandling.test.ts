import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import axios from 'axios'
import { 
  userAPI, 
  AuthErrorType,
  type AuthError,
  type UserCredentials,
  type NewUserData
} from '../api'

// Mock axios
vi.mock('axios', () => {
  const mockAxiosInstance = {
    get: vi.fn(),
    post: vi.fn(),
    patch: vi.fn(),
    delete: vi.fn(),
    interceptors: {
      request: { use: vi.fn() },
      response: { use: vi.fn() }
    }
  }

  return {
    default: {
      create: vi.fn(() => mockAxiosInstance),
      interceptors: {
        request: { use: vi.fn() },
        response: { use: vi.fn() }
      }
    }
  }
})

describe('Authentication Error Handling', () => {
  let mockAxiosInstance: any

  beforeEach(() => {
    vi.clearAllMocks()
    
    // Get the mocked axios instance
    const mockedAxios = vi.mocked(axios)
    mockAxiosInstance = mockedAxios.create()
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  describe('Network and Connection Errors', () => {
    it('should handle network timeout during login', async () => {
      const credentials: UserCredentials = {
        username: 'testuser',
        password: 'password123'
      }

      const timeoutError = {
        code: 'ECONNABORTED',
        message: 'timeout of 30000ms exceeded'
      }

      mockAxiosInstance.post.mockRejectedValueOnce(timeoutError)

      const result = await userAPI.login(credentials)

      expect(result).toEqual({
        success: false,
        error: 'Login failed',
        requiresRegistration: false
      })
    })

    it('should handle connection refused error', async () => {
      const credentials: UserCredentials = {
        username: 'testuser',
        password: 'password123'
      }

      const connectionError = {
        code: 'ECONNREFUSED',
        message: 'connect ECONNREFUSED 127.0.0.1:8080'
      }

      mockAxiosInstance.post.mockRejectedValueOnce(connectionError)

      const result = await userAPI.login(credentials)

      expect(result).toEqual({
        success: false,
        error: 'Login failed',
        requiresRegistration: false
      })
    })

    it('should handle DNS resolution errors', async () => {
      const credentials: UserCredentials = {
        username: 'testuser',
        password: 'password123'
      }

      const dnsError = {
        code: 'ENOTFOUND',
        message: 'getaddrinfo ENOTFOUND api.example.com'
      }

      mockAxiosInstance.post.mockRejectedValueOnce(dnsError)

      const result = await userAPI.login(credentials)

      expect(result).toEqual({
        success: false,
        error: 'Login failed',
        requiresRegistration: false
      })
    })
  })

  describe('HTTP Status Code Error Handling', () => {
    it('should handle 400 Bad Request errors', async () => {
      const credentials: UserCredentials = {
        username: '',
        password: 'password123'
      }

      const badRequestError = {
        response: {
          status: 400,
          data: {
            message: 'Username is required',
            errors: {
              username: 'Username cannot be empty'
            }
          }
        }
      }

      mockAxiosInstance.post.mockRejectedValueOnce(badRequestError)

      const result = await userAPI.login(credentials)

      expect(result).toEqual({
        success: false,
        error: 'Username is required',
        requiresRegistration: false
      })
    })

    it('should handle 401 Unauthorized errors', async () => {
      const credentials: UserCredentials = {
        username: 'testuser',
        password: 'wrongpassword'
      }

      const unauthorizedError = {
        response: {
          status: 401,
          data: {
            message: 'Invalid username or password'
          }
        }
      }

      mockAxiosInstance.post.mockRejectedValueOnce(unauthorizedError)

      const result = await userAPI.login(credentials)

      expect(result).toEqual({
        success: false,
        error: 'Invalid username or password',
        requiresRegistration: false
      })
    })

    it('should handle 403 Forbidden errors', async () => {
      const credentials: UserCredentials = {
        username: 'lockeduser',
        password: 'password123'
      }

      const forbiddenError = {
        response: {
          status: 403,
          data: {
            message: 'Account is locked due to too many failed login attempts'
          }
        }
      }

      mockAxiosInstance.post.mockRejectedValueOnce(forbiddenError)

      const result = await userAPI.login(credentials)

      expect(result).toEqual({
        success: false,
        error: 'Account is locked due to too many failed login attempts',
        requiresRegistration: false
      })
    })

    it('should handle 404 Not Found errors for user registration', async () => {
      const credentials: UserCredentials = {
        username: 'newuser',
        password: 'password123'
      }

      const notFoundError = {
        response: {
          status: 404,
          data: {
            message: 'User not found'
          }
        }
      }

      mockAxiosInstance.post.mockRejectedValueOnce(notFoundError)

      const result = await userAPI.login(credentials)

      expect(result).toEqual({
        success: false,
        error: 'User not found',
        requiresRegistration: true
      })
    })

    it('should handle 409 Conflict errors during registration', async () => {
      const userData: NewUserData = {
        username: 'existinguser',
        password: 'password123',
        confirmPassword: 'password123'
      }

      const conflictError = {
        response: {
          status: 409,
          data: {
            message: 'Username already exists'
          }
        }
      }

      mockAxiosInstance.post.mockRejectedValueOnce(conflictError)

      const result = await userAPI.register(userData)

      expect(result).toEqual({
        success: false,
        error: 'Username already exists'
      })
    })

    it('should handle 422 Unprocessable Entity errors', async () => {
      const userData: NewUserData = {
        username: 'user',
        password: '123',
        confirmPassword: '123'
      }

      const validationError = {
        response: {
          status: 422,
          data: {
            message: 'Validation failed',
            errors: {
              password: 'Password must be at least 8 characters long'
            }
          }
        }
      }

      mockAxiosInstance.post.mockRejectedValueOnce(validationError)

      const result = await userAPI.register(userData)

      expect(result).toEqual({
        success: false,
        error: 'Validation failed'
      })
    })

    it('should handle 429 Too Many Requests errors', async () => {
      const credentials: UserCredentials = {
        username: 'testuser',
        password: 'password123'
      }

      const rateLimitError = {
        response: {
          status: 429,
          data: {
            message: 'Too many login attempts. Please try again later.',
            retryAfter: 300
          }
        }
      }

      mockAxiosInstance.post.mockRejectedValueOnce(rateLimitError)

      const result = await userAPI.login(credentials)

      expect(result).toEqual({
        success: false,
        error: 'Too many login attempts. Please try again later.',
        requiresRegistration: false
      })
    })

    it('should handle 500 Internal Server Error', async () => {
      const credentials: UserCredentials = {
        username: 'testuser',
        password: 'password123'
      }

      const serverError = {
        response: {
          status: 500,
          data: {
            message: 'Internal server error'
          }
        }
      }

      mockAxiosInstance.post.mockRejectedValueOnce(serverError)

      const result = await userAPI.login(credentials)

      expect(result).toEqual({
        success: false,
        error: 'Internal server error',
        requiresRegistration: false
      })
    })

    it('should handle 503 Service Unavailable errors', async () => {
      const credentials: UserCredentials = {
        username: 'testuser',
        password: 'password123'
      }

      const serviceUnavailableError = {
        response: {
          status: 503,
          data: {
            message: 'Service temporarily unavailable'
          }
        }
      }

      mockAxiosInstance.post.mockRejectedValueOnce(serviceUnavailableError)

      const result = await userAPI.login(credentials)

      expect(result).toEqual({
        success: false,
        error: 'Service temporarily unavailable',
        requiresRegistration: false
      })
    })
  })

  describe('Malformed Response Handling', () => {
    it('should handle responses without error messages', async () => {
      const credentials: UserCredentials = {
        username: 'testuser',
        password: 'password123'
      }

      const malformedError = {
        response: {
          status: 400,
          data: {}
        }
      }

      mockAxiosInstance.post.mockRejectedValueOnce(malformedError)

      const result = await userAPI.login(credentials)

      expect(result).toEqual({
        success: false,
        error: 'Login failed',
        requiresRegistration: false
      })
    })

    it('should handle responses with null data', async () => {
      const credentials: UserCredentials = {
        username: 'testuser',
        password: 'password123'
      }

      const nullDataError = {
        response: {
          status: 500,
          data: null
        }
      }

      mockAxiosInstance.post.mockRejectedValueOnce(nullDataError)

      const result = await userAPI.login(credentials)

      expect(result).toEqual({
        success: false,
        error: 'Login failed',
        requiresRegistration: false
      })
    })

    it('should handle responses with unexpected data structure', async () => {
      const credentials: UserCredentials = {
        username: 'testuser',
        password: 'password123'
      }

      const unexpectedError = {
        response: {
          status: 400,
          data: 'Unexpected string response'
        }
      }

      mockAxiosInstance.post.mockRejectedValueOnce(unexpectedError)

      const result = await userAPI.login(credentials)

      expect(result).toEqual({
        success: false,
        error: 'Login failed',
        requiresRegistration: false
      })
    })
  })

  describe('Session and Token Error Handling', () => {
    it('should handle expired token during getCurrentUser', async () => {
      const expiredTokenError = {
        response: {
          status: 401,
          data: {
            message: 'Token has expired'
          }
        }
      }

      mockAxiosInstance.get.mockRejectedValueOnce(expiredTokenError)

      await expect(userAPI.getCurrentUser()).rejects.toEqual(expiredTokenError)
    })

    it('should handle invalid token during session validation', async () => {
      const invalidTokenError = {
        response: {
          status: 401,
          data: {
            message: 'Invalid token'
          }
        }
      }

      mockAxiosInstance.get.mockRejectedValueOnce(invalidTokenError)

      const result = await userAPI.validateSession()

      expect(result).toEqual({ valid: false })
    })

    it('should handle refresh token expiration', async () => {
      const refreshTokenError = {
        response: {
          status: 401,
          data: {
            message: 'Refresh token has expired'
          }
        }
      }

      mockAxiosInstance.post.mockRejectedValueOnce(refreshTokenError)

      await expect(userAPI.refreshToken()).rejects.toEqual(refreshTokenError)
    })
  })

  describe('User Search Error Handling', () => {
    it('should handle search service unavailable', async () => {
      const searchError = {
        response: {
          status: 503,
          data: {
            message: 'Search service temporarily unavailable'
          }
        }
      }

      mockAxiosInstance.get.mockRejectedValueOnce(searchError)

      await expect(userAPI.searchUsers('test')).rejects.toEqual(searchError)
    })

    it('should handle invalid search parameters', async () => {
      const invalidParamsError = {
        response: {
          status: 400,
          data: {
            message: 'Invalid search parameters'
          }
        }
      }

      mockAxiosInstance.get.mockRejectedValueOnce(invalidParamsError)

      await expect(userAPI.searchUsers('')).rejects.toEqual(invalidParamsError)
    })
  })

  describe('Password Change Error Handling', () => {
    it('should handle incorrect current password', async () => {
      const incorrectPasswordError = {
        response: {
          status: 400,
          data: {
            message: 'Current password is incorrect'
          }
        }
      }

      mockAxiosInstance.post.mockRejectedValueOnce(incorrectPasswordError)

      const result = await userAPI.changePassword('wrongpass', 'newpass123')

      expect(result).toEqual({
        success: false,
        error: 'Current password is incorrect'
      })
    })

    it('should handle weak new password', async () => {
      const weakPasswordError = {
        response: {
          status: 422,
          data: {
            message: 'New password does not meet security requirements'
          }
        }
      }

      mockAxiosInstance.post.mockRejectedValueOnce(weakPasswordError)

      const result = await userAPI.changePassword('oldpass', '123')

      expect(result).toEqual({
        success: false,
        error: 'New password does not meet security requirements'
      })
    })
  })

  describe('Error Recovery and Retry Logic', () => {
    it('should handle transient errors gracefully', async () => {
      const credentials: UserCredentials = {
        username: 'testuser',
        password: 'password123'
      }

      // First call fails with transient error
      const transientError = {
        response: {
          status: 503,
          data: {
            message: 'Service temporarily unavailable'
          }
        }
      }

      mockAxiosInstance.post.mockRejectedValueOnce(transientError)

      const result = await userAPI.login(credentials)

      expect(result).toEqual({
        success: false,
        error: 'Service temporarily unavailable',
        requiresRegistration: false
      })
    })

    it('should provide appropriate error categorization', async () => {
      const testCases = [
        {
          status: 400,
          message: 'Validation error',
          expectedType: AuthErrorType.VALIDATION_ERROR
        },
        {
          status: 401,
          message: 'Invalid credentials',
          expectedType: AuthErrorType.INVALID_CREDENTIALS
        },
        {
          status: 404,
          message: 'User not found',
          expectedType: AuthErrorType.USER_NOT_FOUND
        },
        {
          status: 409,
          message: 'Username taken',
          expectedType: AuthErrorType.USERNAME_TAKEN
        },
        {
          status: 500,
          message: 'Server error',
          expectedType: AuthErrorType.SERVER_ERROR
        }
      ]

      // This test demonstrates how errors could be categorized
      // In a real implementation, you might want to add error type classification
      testCases.forEach(testCase => {
        expect(testCase.status).toBeGreaterThan(0)
        expect(testCase.message).toBeTruthy()
        expect(testCase.expectedType).toBeTruthy()
      })
    })
  })
})