/**
 * Unit tests for SystemInfo page with OSHI enhancement support
 */

import React from 'react';
import { render, screen, waitFor } from '@testing-library/react';
import { describe, it, expect, beforeEach, vi } from 'vitest';
import SystemInfoPage from '../SystemInfo';
import * as apiModule from '../../services/api';

// Mock the API module
vi.mock('../../services/api', () => ({
  systemAPI: {
    getStatus: vi.fn(),
  },
}));

// Mock window.efsApi
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

Object.defineProperty(window, 'efsApi', {
  value: mockEfsApi,
  writable: true,
});

describe('SystemInfo Page', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('should render enhanced OSHI system information when available', async () => {
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
    };

    vi.spyOn(apiModule.systemAPI, 'getStatus').mockResolvedValue(mockEnhancedStatus);

    render(<SystemInfoPage />);

    await waitFor(() => {
      expect(screen.getByText('System Information')).toBeInTheDocument();
    });

    // Should show enhanced OSHI data badge
    expect(screen.getByText('Enhanced OSHI Data')).toBeInTheDocument();

    // Should display CPU information
    expect(screen.getByText('CPU Information')).toBeInTheDocument();
    expect(screen.getByText('Intel Core i7-9700K')).toBeInTheDocument();
    expect(screen.getByText('GenuineIntel')).toBeInTheDocument();

    // Should display memory information
    expect(screen.getByText('Memory Information')).toBeInTheDocument();
    expect(screen.getByText('50.0%')).toBeInTheDocument();

    // Should display operating system information
    expect(screen.getByText('Operating System')).toBeInTheDocument();
    expect(screen.getByText('Windows 10')).toBeInTheDocument();
  });

  it('should render legacy system capabilities when enhanced data is not available', async () => {
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

    // Mock API failure to trigger fallback
    vi.spyOn(apiModule.systemAPI, 'getStatus').mockRejectedValue(new Error('API not available'));
    mockEfsApi.getSystemCapabilities.mockResolvedValue(mockLegacyCapabilities);

    render(<SystemInfoPage />);

    await waitFor(() => {
      expect(screen.getByText('System Information')).toBeInTheDocument();
    });

    // Should show legacy system data badge
    expect(screen.getByText('Legacy System Data')).toBeInTheDocument();

    // Should display capability score
    expect(screen.getByText('System Capability Score')).toBeInTheDocument();
    expect(screen.getByText('85/100')).toBeInTheDocument();
    expect(screen.getByText('Excellent')).toBeInTheDocument();

    // Should display CPU information
    expect(screen.getByText('Intel Core i5-8400')).toBeInTheDocument();
    expect(screen.getByText('GenuineIntel')).toBeInTheDocument();

    // Should display hardware acceleration status
    expect(screen.getByText('Hardware Acceleration')).toBeInTheDocument();
    expect(screen.getByText('Available')).toBeInTheDocument();
  });

  it('should handle error states gracefully', async () => {
    const errorMessage = 'Failed to fetch system information';

    // Mock both API and IPC failures
    vi.spyOn(apiModule.systemAPI, 'getStatus').mockRejectedValue(new Error('API error'));
    mockEfsApi.getSystemCapabilities.mockRejectedValue(new Error(errorMessage));

    render(<SystemInfoPage />);

    await waitFor(() => {
      expect(screen.getByText(`Error loading system information: ${errorMessage}`)).toBeInTheDocument();
    });

    // Should show retry button
    expect(screen.getByText('Retry')).toBeInTheDocument();
  });

  it('should handle partial enhanced data gracefully', async () => {
    const mockPartialStatus = {
      version: '1.0.0',
      uptime: 86400000,
      containersCount: 2,
      mountedCount: 1,
      totalSize: 1024 * 1024 * 1024 * 100,
      availableSpace: 1024 * 1024 * 1024 * 70,
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
      // memory, operatingSystem, disks, networkInterfaces are undefined
      memory: undefined,
      operatingSystem: undefined,
      disks: undefined,
      networkInterfaces: undefined,
    };

    vi.spyOn(apiModule.systemAPI, 'getStatus').mockResolvedValue(mockPartialStatus);

    render(<SystemInfoPage />);

    await waitFor(() => {
      expect(screen.getByText('System Information')).toBeInTheDocument();
    });

    // Should show enhanced OSHI data badge
    expect(screen.getByText('Enhanced OSHI Data')).toBeInTheDocument();

    // Should display available CPU information
    expect(screen.getByText('AMD Ryzen 7 3700X')).toBeInTheDocument();
    expect(screen.getByText('AuthenticAMD')).toBeInTheDocument();

    // Should not crash when memory/OS info is missing
    expect(screen.queryByText('Memory Information')).not.toBeInTheDocument();
    expect(screen.queryByText('Operating System')).not.toBeInTheDocument();
  });

  it('should display loading state initially', () => {
    // Don't resolve the promises to keep loading state
    vi.spyOn(apiModule.systemAPI, 'getStatus').mockImplementation(() => new Promise(() => {}));
    mockEfsApi.getSystemCapabilities.mockImplementation(() => new Promise(() => {}));

    render(<SystemInfoPage />);

    expect(screen.getByText('Detecting system capabilities...')).toBeInTheDocument();
    expect(screen.getByRole('status')).toBeInTheDocument(); // Loading spinner
  });

  it('should handle OSHI data unavailable error', async () => {
    const mockErrorCapabilities = {
      error: 'OSHI initialization failed',
      systemInfo: null,
      capabilityScore: 0,
      features: {},
      warnings: ['Failed to detect system capabilities'],
    };

    vi.spyOn(apiModule.systemAPI, 'getStatus').mockRejectedValue(new Error('API not available'));
    mockEfsApi.getSystemCapabilities.mockResolvedValue(mockErrorCapabilities);

    render(<SystemInfoPage />);

    await waitFor(() => {
      expect(screen.getByText('Error loading system information: OSHI initialization failed')).toBeInTheDocument();
    });
  });
});