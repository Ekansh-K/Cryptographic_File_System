/**
 * Integration tests for System Information with OSHI enhancement
 * Tests frontend compatibility with enhanced backend responses
 */

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { systemAPI } from '../services/api';

// Mock the window.efsApi
const mockEfsApi = {
  getSystemCapabilities: vi.fn(),
  getEnhancedSystemStatus: vi.fn(),
  getCpuInfo: vi.fn(),
  mountContainer: vi.fn(),
  unmountContainer: vi.fn(),
  getMountStatus: vi.fn(),
  checkHardwareAcceleration: vi.fn(),
  checkNativeDrivers: vi.fn(),
};

// Mock window object
Object.defineProperty(window, 'efsApi', {
  value: mockEfsApi,
  writable: true,
});

describe('System Information Integration', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe('Enhanced OSHI System Status', () => {
    it('should handle enhanced system status from backend API', async () => {
      const mockEnhancedStatus = {
        version: '1.0.0',
        uptime: 86400000,
        containersCount: 5,
        mountedCount: 2,
        totalSize: 1024 * 1024 * 1024 * 100,
        availableSpace: 1024 * 1024 * 1024 * 50,
        cpu: {
          modelName: 'Intel Core i7-9700K',
          vendor: 'GenuineIntel',
          physicalCores: 8,
          logicalCores: 8,
          baseFrequency: 3600,
          maxFrequency: 4900,
          features: {
            hasAesNi: true,
            hasAvx: true,
            hasAvx2: true,
            hasRdrand: true,
            hasRdseed: false,
            hasVaes: false,
          },
          cache: {
            l1Cache: 32768,
            l2Cache: 262144,
            l3Cache: 12582912,
          },
          currentUsage: 25.5,
        },
        memory: {
          totalPhysical: 17179869184,
          availablePhysical: 8589934592,
          usedPhysical: 8589934592,
          totalVirtual: 25769803776,
          availableVirtual: 12884901888,
          usedVirtual: 12884901888,
          usagePercentage: 50.0,
        },
        operatingSystem: {
          name: 'Windows 10',
          version: '10.0.19045',
          buildNumber: '19045',
          architecture: 'amd64',
          isVirtualMachine: false,
          bootTime: 1640995200000,
          processCount: 150,
          threadCount: 1500,
        },
        disks: [
          {
            name: 'C:',
            model: 'Samsung SSD 970 EVO Plus 1TB',
            serial: 'S4EWNX0R123456',
            size: 1024 * 1024 * 1024 * 1000,
            reads: 1000000,
            writes: 500000,
            readBytes: 1024 * 1024 * 1024 * 10,
            writeBytes: 1024 * 1024 * 1024 * 5,
            transferTime: 5000,
          },
        ],
        networkInterfaces: [
          {
            name: 'Ethernet',
            displayName: 'Intel Ethernet Connection',
            macAddress: '00:11:22:33:44:55',
            ipv4Addresses: ['192.168.1.100'],
            ipv6Addresses: ['fe80::1'],
            bytesReceived: 1024 * 1024 * 1024,
            bytesSent: 1024 * 1024 * 512,
            packetsReceived: 1000000,
            packetsSent: 500000,
            speed: 1000000000,
            isUp: true,
          },
        ],
      };

      // Mock successful API response
      vi.spyOn(systemAPI, 'getStatus').mockResolvedValue(mockEnhancedStatus);

      const result = await systemAPI.getStatus();

      expect(result).toEqual(mockEnhancedStatus);
      expect(result.cpu).toBeDefined();
      expect(result.memory).toBeDefined();
      expect(result.operatingSystem).toBeDefined();
      expect(result.disks).toBeDefined();
      expect(result.networkInterfaces).toBeDefined();
    });

    it('should handle enhanced system status from IPC when backend fails', async () => {
      const mockEnhancedStatus = {
        version: '1.0.0',
        uptime: 86400000,
        containersCount: 3,
        mountedCount: 1,
        totalSize: 1024 * 1024 * 1024 * 100,
        availableSpace: 1024 * 1024 * 1024 * 60,
        cpu: {
          modelName: 'AMD Ryzen 7 3700X',
          vendor: 'AuthenticAMD',
          physicalCores: 8,
          logicalCores: 16,
          baseFrequency: 3600,
          maxFrequency: 4400,
          features: {
            hasAesNi: true,
            hasAvx: true,
            hasAvx2: true,
            hasRdrand: true,
            hasRdseed: true,
            hasVaes: false,
          },
          cache: {
            l1Cache: 65536,
            l2Cache: 524288,
            l3Cache: 33554432,
          },
          currentUsage: 15.2,
        },
        memory: {
          totalPhysical: 34359738368,
          availablePhysical: 17179869184,
          usedPhysical: 17179869184,
          totalVirtual: 51539607552,
          availableVirtual: 25769803776,
          usedVirtual: 25769803776,
          usagePercentage: 50.0,
        },
      };

      // Mock backend API failure
      vi.spyOn(systemAPI, 'getStatus').mockRejectedValue(new Error('Backend not available'));
      
      // Mock IPC success
      mockEfsApi.getEnhancedSystemStatus.mockResolvedValue(mockEnhancedStatus);

      const result = await mockEfsApi.getEnhancedSystemStatus();

      expect(result).toEqual(mockEnhancedStatus);
      expect(result.cpu).toBeDefined();
      expect(result.memory).toBeDefined();
    });

    it('should gracefully fallback to legacy system capabilities', async () => {
      const mockLegacyCapabilities = {
        systemInfo: {
          cpu: {
            modelName: 'Intel Core i5-8400',
            vendor: 'GenuineIntel',
            coreCount: 6,
            logicalProcessors: 6,
            hasAesNi: true,
            hasAvx: true,
            hasAvx2: true,
            hasVaes: false,
            hasRdrand: true,
            hasRdseed: false,
            cacheL1: 32768,
            cacheL2: 262144,
            cacheL3: 9437184,
            baseFrequency: 2800,
            maxFrequency: 4000,
          },
          totalMemory: 8589934592,
          availableMemory: 4294967296,
          osName: 'Windows 10',
          osVersion: '10.0.19045',
          architecture: 'amd64',
          isVirtualMachine: false,
          winfspAvailable: true,
          fuseAvailable: false,
        },
        capabilityScore: 85,
        features: {
          fileLocking: true,
          symbolicLinks: true,
          accessControlLists: true,
          extendedAttributes: false,
          filesystemMounting: true,
          hardwareAes: true,
        },
        warnings: [],
      };

      // Mock both enhanced methods failing
      vi.spyOn(systemAPI, 'getStatus').mockRejectedValue(new Error('Backend not available'));
      mockEfsApi.getEnhancedSystemStatus.mockRejectedValue(new Error('Enhanced status not available'));
      
      // Mock legacy capabilities success
      mockEfsApi.getSystemCapabilities.mockResolvedValue(mockLegacyCapabilities);

      const result = await mockEfsApi.getSystemCapabilities();

      expect(result).toEqual(mockLegacyCapabilities);
      expect(result.systemInfo).toBeDefined();
      expect(result.capabilityScore).toBe(85);
      expect(result.features.hardwareAes).toBe(true);
    });
  });

  describe('Error Handling', () => {
    it('should handle OSHI data unavailable gracefully', async () => {
      const mockErrorResponse = {
        error: 'OSHI initialization failed',
        systemInfo: null,
        capabilityScore: 0,
        features: {},
        warnings: ['Failed to detect system capabilities'],
      };

      mockEfsApi.getSystemCapabilities.mockResolvedValue(mockErrorResponse);

      const result = await mockEfsApi.getSystemCapabilities();

      expect(result.error).toBeDefined();
      expect(result.warnings).toContain('Failed to detect system capabilities');
    });

    it('should handle partial OSHI data availability', async () => {
      const mockPartialStatus = {
        version: '1.0.0',
        uptime: 86400000,
        containersCount: 2,
        mountedCount: 1,
        totalSize: 1024 * 1024 * 1024 * 100,
        availableSpace: 1024 * 1024 * 1024 * 70,
        cpu: {
          modelName: 'Intel Core i3-8100',
          vendor: 'GenuineIntel',
          physicalCores: 4,
          logicalCores: 4,
          baseFrequency: 3600,
          maxFrequency: 3600,
          features: {
            hasAesNi: true,
            hasAvx: false,
            hasAvx2: false,
            hasRdrand: true,
            hasRdseed: false,
            hasVaes: false,
          },
          cache: {
            l1Cache: 32768,
            l2Cache: 262144,
            l3Cache: 6291456,
          },
          currentUsage: 10.5,
        },
        // memory and operatingSystem are undefined (partial data)
        memory: undefined,
        operatingSystem: undefined,
        disks: undefined,
        networkInterfaces: undefined,
      };

      mockEfsApi.getEnhancedSystemStatus.mockResolvedValue(mockPartialStatus);

      const result = await mockEfsApi.getEnhancedSystemStatus();

      expect(result.cpu).toBeDefined();
      expect(result.memory).toBeUndefined();
      expect(result.operatingSystem).toBeUndefined();
      // Should still be usable with partial data
      expect(result.version).toBe('1.0.0');
    });

    it('should handle network timeout gracefully', async () => {
      const timeoutError = new Error('Request timeout');
      timeoutError.name = 'TimeoutError';

      vi.spyOn(systemAPI, 'getStatus').mockRejectedValue(timeoutError);
      mockEfsApi.getEnhancedSystemStatus.mockRejectedValue(timeoutError);
      
      // Should fallback to legacy
      const mockLegacyCapabilities = {
        systemInfo: {
          cpu: {
            modelName: 'Unknown Processor',
            vendor: 'Unknown',
            coreCount: 1,
            logicalProcessors: 1,
            hasAesNi: false,
            hasAvx: false,
            hasAvx2: false,
            hasVaes: false,
            hasRdrand: false,
            hasRdseed: false,
            cacheL1: 0,
            cacheL2: 0,
            cacheL3: 0,
            baseFrequency: 0,
            maxFrequency: 0,
          },
          totalMemory: 1073741824,
          availableMemory: 536870912,
          osName: 'Unknown',
          osVersion: 'Unknown',
          architecture: 'unknown',
          isVirtualMachine: false,
          winfspAvailable: false,
          fuseAvailable: false,
        },
        capabilityScore: 20,
        features: {
          fileLocking: false,
          symbolicLinks: false,
          accessControlLists: false,
          extendedAttributes: false,
          filesystemMounting: false,
          hardwareAes: false,
        },
        warnings: ['System detection failed due to timeout'],
      };

      mockEfsApi.getSystemCapabilities.mockResolvedValue(mockLegacyCapabilities);

      const result = await mockEfsApi.getSystemCapabilities();

      expect(result.capabilityScore).toBe(20);
      expect(result.warnings).toContain('System detection failed due to timeout');
    });
  });

  describe('Data Format Compatibility', () => {
    it('should maintain backward compatibility with existing SystemStatus interface', async () => {
      const mockStatus = {
        version: '1.0.0',
        uptime: 86400000,
        containersCount: 5,
        mountedCount: 2,
        totalSize: 1024 * 1024 * 1024 * 100,
        availableSpace: 1024 * 1024 * 1024 * 50,
        // Enhanced fields are optional
        cpu: undefined,
        memory: undefined,
        operatingSystem: undefined,
        disks: undefined,
        networkInterfaces: undefined,
      };

      vi.spyOn(systemAPI, 'getStatus').mockResolvedValue(mockStatus);

      const result = await systemAPI.getStatus();

      // Should have all required legacy fields
      expect(result.version).toBeDefined();
      expect(result.uptime).toBeDefined();
      expect(result.containersCount).toBeDefined();
      expect(result.mountedCount).toBeDefined();
      expect(result.totalSize).toBeDefined();
      expect(result.availableSpace).toBeDefined();

      // Enhanced fields can be undefined
      expect(result.cpu).toBeUndefined();
      expect(result.memory).toBeUndefined();
    });

    it('should handle mixed legacy and enhanced data', async () => {
      const mockMixedStatus = {
        // Legacy fields
        version: '1.0.0',
        uptime: 86400000,
        containersCount: 3,
        mountedCount: 1,
        totalSize: 1024 * 1024 * 1024 * 100,
        availableSpace: 1024 * 1024 * 1024 * 60,
        
        // Some enhanced fields available
        cpu: {
          modelName: 'Intel Core i7-8700K',
          vendor: 'GenuineIntel',
          physicalCores: 6,
          logicalCores: 12,
          baseFrequency: 3700,
          maxFrequency: 4700,
          features: {
            hasAesNi: true,
            hasAvx: true,
            hasAvx2: true,
            hasRdrand: true,
            hasRdseed: false,
            hasVaes: false,
          },
          cache: {
            l1Cache: 32768,
            l2Cache: 262144,
            l3Cache: 12582912,
          },
          currentUsage: 20.0,
        },
        
        // Other enhanced fields not available
        memory: undefined,
        operatingSystem: undefined,
        disks: undefined,
        networkInterfaces: undefined,
      };

      mockEfsApi.getEnhancedSystemStatus.mockResolvedValue(mockMixedStatus);

      const result = await mockEfsApi.getEnhancedSystemStatus();

      expect(result.version).toBe('1.0.0');
      expect(result.cpu).toBeDefined();
      expect(result.cpu.modelName).toBe('Intel Core i7-8700K');
      expect(result.memory).toBeUndefined();
    });
  });
});