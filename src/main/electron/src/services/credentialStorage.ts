// Use Web Crypto API for browser compatibility in Electron renderer process

// Types for credential storage
export interface StoredCredentials {
  encryptedData: string
  salt: string
  iv: string
  timestamp: number
  version: string
}

export interface UserCredentials {
  username: string
  password: string
}

export interface DeviceInfo {
  platform: string
  arch: string
  hostname: string
  userAgent: string
}

/**
 * Secure credential storage service using AES-256-GCM encryption
 * with device-specific key derivation using PBKDF2
 */
export class SecureCredentialStorage {
  private readonly ENCRYPTION_ALGORITHM = 'aes-256-gcm'
  private readonly KEY_DERIVATION_ITERATIONS = 100000
  private readonly STORAGE_VERSION = '1.0'
  private readonly STORAGE_PREFIX = 'efs_cred_'

  /**
   * Store user credentials securely with AES-256 encryption
   */
  async storeCredentials(username: string, password: string): Promise<void> {
    try {
      const salt = this.generateSalt()
      const deviceKey = await this.getDeviceKey()
      const derivedKey = await this.deriveKey(deviceKey, salt)

      const credentials: UserCredentials = { username, password }
      const encryptedData = await this.encrypt(JSON.stringify(credentials), derivedKey)

      const storedData: StoredCredentials = {
        encryptedData: encryptedData.encrypted,
        salt: Array.from(salt).map(b => b.toString(16).padStart(2, '0')).join(''),
        iv: encryptedData.iv,
        timestamp: Date.now(),
        version: this.STORAGE_VERSION
      }

      localStorage.setItem(
        this.getStorageKey(username),
        JSON.stringify(storedData)
      )
    } catch (error) {
      throw new Error(`Failed to store credentials: ${error instanceof Error ? error.message : 'Unknown error'}`)
    }
  }

  /**
   * Retrieve and decrypt stored credentials
   */
  async retrieveCredentials(username: string): Promise<UserCredentials | null> {
    try {
      const storedDataStr = localStorage.getItem(this.getStorageKey(username))
      if (!storedDataStr) {
        return null
      }

      const storedData: StoredCredentials = JSON.parse(storedDataStr)

      // Verify storage version compatibility
      if (storedData.version !== this.STORAGE_VERSION) {
        throw new Error('Incompatible credential storage version')
      }

      const deviceKey = await this.getDeviceKey()
      const derivedKey = await this.deriveKey(deviceKey, new Uint8Array(
        storedData.salt.match(/.{1,2}/g)!.map(byte => parseInt(byte, 16))
      ))

      const decryptedData = await this.decrypt({
        encrypted: storedData.encryptedData,
        iv: storedData.iv
      }, derivedKey)

      return JSON.parse(decryptedData) as UserCredentials
    } catch (error) {
      throw new Error(`Failed to retrieve credentials: ${error instanceof Error ? error.message : 'Unknown error'}`)
    }
  }

  /**
   * Delete stored credentials
   */
  async deleteCredentials(username: string): Promise<void> {
    localStorage.removeItem(this.getStorageKey(username))
  }

  /**
   * Verify integrity of stored credentials
   */
  async verifyIntegrity(username: string): Promise<boolean> {
    try {
      const credentials = await this.retrieveCredentials(username)
      return credentials !== null &&
        typeof credentials.username === 'string' &&
        typeof credentials.password === 'string'
    } catch (error) {
      return false
    }
  }

  /**
   * List all stored usernames
   */
  getStoredUsernames(): string[] {
    const usernames: string[] = []
    for (let i = 0; i < localStorage.length; i++) {
      const key = localStorage.key(i)
      if (key && key.startsWith(this.STORAGE_PREFIX)) {
        const username = key.substring(this.STORAGE_PREFIX.length)
        usernames.push(username)
      }
    }
    return usernames
  }

  /**
   * Clear all stored credentials
   */
  clearAllCredentials(): void {
    const keys = []
    for (let i = 0; i < localStorage.length; i++) {
      const key = localStorage.key(i)
      if (key && key.startsWith(this.STORAGE_PREFIX)) {
        keys.push(key)
      }
    }
    keys.forEach(key => localStorage.removeItem(key))
  }

  /**
   * Generate cryptographically secure salt
   */
  private generateSalt(): Uint8Array {
    return crypto.getRandomValues(new Uint8Array(32))
  }

  /**
   * Get device-specific key based on hardware identifiers
   */
  private async getDeviceKey(): Promise<string> {
    const deviceInfo = await this.getDeviceInfo()
    const deviceString = JSON.stringify(deviceInfo)

    // Use Web Crypto API for hashing
    const encoder = new TextEncoder()
    const data = encoder.encode(deviceString)
    const hashBuffer = await crypto.subtle.digest('SHA-256', data)
    const hashArray = new Uint8Array(hashBuffer)

    // Convert to hex string
    return Array.from(hashArray)
      .map(b => b.toString(16).padStart(2, '0'))
      .join('')
  }

  /**
   * Get device information for key derivation
   */
  private async getDeviceInfo(): Promise<DeviceInfo> {
    return {
      platform: navigator.platform,
      arch: navigator.userAgent,
      hostname: window.location.hostname,
      userAgent: navigator.userAgent
    }
  }

  /**
   * Derive encryption key using PBKDF2
   */
  private async deriveKey(deviceKey: string, salt: Uint8Array): Promise<CryptoKey> {
    const encoder = new TextEncoder()
    const keyMaterial = await crypto.subtle.importKey(
      'raw',
      encoder.encode(deviceKey),
      'PBKDF2',
      false,
      ['deriveBits', 'deriveKey']
    )

    return crypto.subtle.deriveKey(
      {
        name: 'PBKDF2',
        salt: salt,
        iterations: this.KEY_DERIVATION_ITERATIONS,
        hash: 'SHA-256'
      },
      keyMaterial,
      { name: 'AES-GCM', length: 256 },
      true,
      ['encrypt', 'decrypt']
    )
  }

  /**
   * Encrypt data using AES-256-GCM
   */
  private async encrypt(data: string, key: CryptoKey): Promise<{ encrypted: string; iv: string }> {
    const encoder = new TextEncoder()
    const iv = crypto.getRandomValues(new Uint8Array(12)) // 12 bytes for GCM

    const encryptedBuffer = await crypto.subtle.encrypt(
      {
        name: 'AES-GCM',
        iv: iv
      },
      key,
      encoder.encode(data)
    )

    const encryptedArray = new Uint8Array(encryptedBuffer)

    return {
      encrypted: Array.from(encryptedArray)
        .map(b => b.toString(16).padStart(2, '0'))
        .join(''),
      iv: Array.from(iv)
        .map(b => b.toString(16).padStart(2, '0'))
        .join('')
    }
  }

  /**
   * Decrypt data using AES-256-GCM
   */
  private async decrypt(encryptedData: { encrypted: string; iv: string }, key: CryptoKey): Promise<string> {
    // Convert hex strings back to Uint8Array
    const encryptedBytes = new Uint8Array(
      encryptedData.encrypted.match(/.{1,2}/g)!.map(byte => parseInt(byte, 16))
    )
    const ivBytes = new Uint8Array(
      encryptedData.iv.match(/.{1,2}/g)!.map(byte => parseInt(byte, 16))
    )

    const decryptedBuffer = await crypto.subtle.decrypt(
      {
        name: 'AES-GCM',
        iv: ivBytes
      },
      key,
      encryptedBytes
    )

    const decoder = new TextDecoder()
    return decoder.decode(decryptedBuffer)
  }

  /**
   * Get storage key for username
   */
  private getStorageKey(username: string): string {
    return `${this.STORAGE_PREFIX}${username}`
  }
}

// Export singleton instance
export const credentialStorage = new SecureCredentialStorage()