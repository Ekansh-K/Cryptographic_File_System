import React, { useState, useEffect, useCallback } from 'react';
import { Card, CardContent, CardHeader, CardTitle } from '../components/ui/card';
import { Badge } from '../components/ui/badge';
import { Alert, AlertDescription } from '../components/ui/alert';
import { Loader2, Cpu, HardDrive, Shield, AlertTriangle, CheckCircle, XCircle, RefreshCw, Monitor, Network } from 'lucide-react';
import { systemAPI, SystemStatus } from '../services/api';

// Cache configuration
const CACHE_DURATION = 5 * 60 * 1000; // 5 minutes
const CACHE_KEY_SYSTEM_INFO = 'system_info_data';

interface CacheEntry<T> {
  data: T;
  timestamp: number;
}

// Cache utilities
const getFromCache = <T,>(key: string): T | null => {
  try {
    const cached = sessionStorage.getItem(key);
    if (!cached) return null;
    
    const entry: CacheEntry<T> = JSON.parse(cached);
    const now = Date.now();
    
    if (now - entry.timestamp > CACHE_DURATION) {
      sessionStorage.removeItem(key);
      return null;
    }
    
    return entry.data;
  } catch (error) {
    console.warn('Error reading from cache:', error);
    return null;
  }
};

const setToCache = <T,>(key: string, data: T): void => {
  try {
    const entry: CacheEntry<T> = {
      data,
      timestamp: Date.now()
    };
    sessionStorage.setItem(key, JSON.stringify(entry));
  } catch (error) {
    console.warn('Error writing to cache:', error);
  }
};

const SystemInfoPage: React.FC = () => {
  const [systemStatus, setSystemStatus] = useState<SystemStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [dataSource, setDataSource] = useState<'backend' | 'ipc' | null>(null);

  useEffect(() => {
    loadSystemInformation();
  }, []);

  const loadSystemInformation = useCallback(async (forceRefresh = false) => {
    try {
      if (!forceRefresh) {
        setLoading(true);
      } else {
        setRefreshing(true);
      }
      setError(null);
      
      // Try to load from cache first (unless force refresh)
      if (!forceRefresh) {
        const cachedData = getFromCache<SystemStatus>(CACHE_KEY_SYSTEM_INFO);
        if (cachedData) {
          setSystemStatus(cachedData);
          setLoading(false);
          return;
        }
      }
      
      // Try to get enhanced system status from backend API first
      try {
        console.log('Attempting to fetch from backend API...');
        const enhancedStatus = await systemAPI.getStatus();
        console.log('Backend API response:', enhancedStatus);
        
        if (enhancedStatus && (enhancedStatus.cpu || enhancedStatus.memory || enhancedStatus.operatingSystem)) {
          setSystemStatus(enhancedStatus);
          setDataSource('backend');
          setToCache(CACHE_KEY_SYSTEM_INFO, enhancedStatus);
          console.log('Successfully loaded data from backend API');
          return;
        }
      } catch (apiError) {
        console.warn('Backend API not available, trying IPC enhanced status:', apiError);
      }

      // Try IPC enhanced system status as fallback
      // @ts-ignore - efsApi is injected by preload script
      if (window.efsApi && window.efsApi.getEnhancedSystemStatus) {
        try {
          console.log('Attempting to fetch from IPC...');
          const enhancedResult = await window.efsApi.getEnhancedSystemStatus();
          console.log('IPC response:', enhancedResult);
          
          if (enhancedResult && !enhancedResult.error && (enhancedResult.cpu || enhancedResult.memory)) {
            setSystemStatus(enhancedResult);
            setDataSource('ipc');
            setToCache(CACHE_KEY_SYSTEM_INFO, enhancedResult);
            console.log('Successfully loaded data from IPC');
            return;
          }
        } catch (ipcError) {
          console.warn('IPC enhanced system status failed:', ipcError);
        }
      }

      // If no data source is available, show error
      setError('System information not available. Please ensure the backend service is running or restart the application.');
    } catch (err) {
      console.error('Error fetching system information:', err);
      setError(err instanceof Error ? err.message : 'Failed to fetch system information');
    } finally {
      setLoading(false);
      setRefreshing(false);
    }
  }, []);

  const handleRefresh = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    loadSystemInformation(true);
  }, [loadSystemInformation]);

  const formatBytes = (bytes: number): string => {
    if (!bytes || bytes === 0) return '0 B';
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    let size = bytes;
    let unitIndex = 0;
    
    while (size >= 1024 && unitIndex < units.length - 1) {
      size /= 1024;
      unitIndex++;
    }
    
    return `${size.toFixed(1)} ${units[unitIndex]}`;
  };

  const formatUptime = (seconds: number): string => {
    if (!seconds) return '0m';
    const hours = Math.floor(seconds / 3600);
    const minutes = Math.floor((seconds % 3600) / 60);
    
    if (hours > 0) {
      return `${hours}h ${minutes}m`;
    }
    return `${minutes}m`;
  };

  if (loading) {
    return (
      <div className="flex items-center justify-center h-64">
        <Loader2 className="h-8 w-8 animate-spin text-blue-600 dark:text-blue-400" />
        <span className="ml-2 text-gray-700 dark:text-gray-300">Loading system information...</span>
      </div>
    );
  }

  if (error) {
    return (
      <Alert className="m-4">
        <AlertTriangle className="h-4 w-4" />
        <AlertDescription>
          {error}
          <button 
            onClick={() => loadSystemInformation(true)}
            className="ml-2 text-blue-600 hover:text-blue-800 dark:text-blue-400 dark:hover:text-blue-300 underline"
          >
            Retry
          </button>
        </AlertDescription>
      </Alert>
    );
  }

  if (!systemStatus) {
    return (
      <Alert className="m-4">
        <AlertTriangle className="h-4 w-4" />
        <AlertDescription>No system information available</AlertDescription>
      </Alert>
    );
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <Badge variant="default">
            {dataSource === 'backend' ? 'Backend API' : 'Local Detection'}
          </Badge>
          {dataSource === 'backend' && (
            <Badge variant="outline">OSHI Enhanced</Badge>
          )}
        </div>
        <button
          onClick={handleRefresh}
          disabled={refreshing}
          className="glass-button px-4 py-2 text-blue-600 dark:text-blue-400 font-medium hover:text-blue-700 dark:hover:text-blue-300 transition-all duration-300 disabled:opacity-50 disabled:cursor-not-allowed flex items-center gap-2"
          type="button"
        >
          {refreshing ? (
            <>
              <Loader2 className="h-4 w-4 animate-spin" />
              Refreshing...
            </>
          ) : (
            <>
              <RefreshCw className="h-4 w-4" />
              Refresh
            </>
          )}
        </button>
      </div>

      {/* System Overview */}
      <Card>
        <CardHeader>
          <CardTitle>System Overview</CardTitle>
        </CardHeader>
        <CardContent>
          <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
            <div>
              <div className="font-semibold text-gray-900 dark:text-gray-100">Version</div>
              <div className="text-sm text-gray-600 dark:text-gray-400">{systemStatus.version || 'N/A'}</div>
            </div>
            <div>
              <div className="font-semibold text-gray-900 dark:text-gray-100">Containers</div>
              <div className="text-sm text-gray-600 dark:text-gray-400">{systemStatus.containersCount || 0} total</div>
            </div>
            <div>
              <div className="font-semibold text-gray-900 dark:text-gray-100">Uptime</div>
              <div className="text-sm text-gray-600 dark:text-gray-400">{formatUptime(systemStatus.uptime || 0)}</div>
            </div>
            <div>
              <div className="font-semibold text-gray-900 dark:text-gray-100">Available Space</div>
              <div className="text-sm text-gray-600 dark:text-gray-400">{formatBytes(systemStatus.availableSpace || 0)}</div>
            </div>
          </div>
        </CardContent>
      </Card>

      {/* CPU Information */}
      {systemStatus.cpu && (
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center">
              <Cpu className="h-5 w-5 mr-2" />
              CPU Information
            </CardTitle>
          </CardHeader>
          <CardContent>
            <div className="space-y-3">
              <div className="glass-card p-3 rounded-lg bg-gray-50/50 dark:bg-gray-800/50 border border-gray-200/30 dark:border-gray-700/30">
                <div className="font-semibold text-gray-900 dark:text-gray-100">Model</div>
                <div className="text-sm text-gray-600 dark:text-gray-400">{systemStatus.cpu.modelName || 'Unknown'}</div>
              </div>
              <div className="glass-card p-3 rounded-lg bg-gray-50/50 dark:bg-gray-800/50 border border-gray-200/30 dark:border-gray-700/30">
                <div className="font-semibold text-gray-900 dark:text-gray-100">Vendor</div>
                <div className="text-sm text-gray-600 dark:text-gray-400">{systemStatus.cpu.vendor || 'Unknown'}</div>
              </div>
              <div className="grid grid-cols-2 gap-3">
                <div className="glass-card p-3 rounded-lg bg-gray-50/50 dark:bg-gray-800/50 border border-gray-200/30 dark:border-gray-700/30">
                  <div className="font-semibold text-gray-900 dark:text-gray-100">Physical Cores</div>
                  <div className="text-sm text-gray-600 dark:text-gray-400">{systemStatus.cpu.physicalCores || 'N/A'}</div>
                </div>
                <div className="glass-card p-3 rounded-lg bg-gray-50/50 dark:bg-gray-800/50 border border-gray-200/30 dark:border-gray-700/30">
                  <div className="font-semibold text-gray-900 dark:text-gray-100">Logical Cores</div>
                  <div className="text-sm text-gray-600 dark:text-gray-400">{systemStatus.cpu.logicalCores || 'N/A'}</div>
                </div>
              </div>
              {systemStatus.cpu.features && (
                <div className="glass-card p-3 rounded-lg bg-gray-50/50 dark:bg-gray-800/50 border border-gray-200/30 dark:border-gray-700/30">
                  <div className="font-semibold mb-2 text-gray-900 dark:text-gray-100">CPU Features</div>
                  <div className="flex flex-wrap gap-2">
                    <Badge variant={systemStatus.cpu.features.hasAesNi ? "default" : "secondary"}>
                      {systemStatus.cpu.features.hasAesNi ? <CheckCircle className="h-3 w-3 mr-1" /> : <XCircle className="h-3 w-3 mr-1" />}
                      AES-NI
                    </Badge>
                    <Badge variant={systemStatus.cpu.features.hasAvx ? "default" : "secondary"}>
                      {systemStatus.cpu.features.hasAvx ? <CheckCircle className="h-3 w-3 mr-1" /> : <XCircle className="h-3 w-3 mr-1" />}
                      AVX
                    </Badge>
                    <Badge variant={systemStatus.cpu.features.hasAvx2 ? "default" : "secondary"}>
                      {systemStatus.cpu.features.hasAvx2 ? <CheckCircle className="h-3 w-3 mr-1" /> : <XCircle className="h-3 w-3 mr-1" />}
                      AVX2
                    </Badge>
                    <Badge variant={systemStatus.cpu.features.hasRdrand ? "default" : "secondary"}>
                      {systemStatus.cpu.features.hasRdrand ? <CheckCircle className="h-3 w-3 mr-1" /> : <XCircle className="h-3 w-3 mr-1" />}
                      RDRAND
                    </Badge>
                  </div>
                </div>
              )}
            </div>
          </CardContent>
        </Card>
      )}

      {/* Memory Information */}
      {systemStatus.memory && (
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center">
              <HardDrive className="h-5 w-5 mr-2" />
              Memory Information
            </CardTitle>
          </CardHeader>
          <CardContent>
            <div className="grid grid-cols-2 gap-4">
              <div className="glass-card p-3 rounded-lg bg-gray-50/50 dark:bg-gray-800/50 border border-gray-200/30 dark:border-gray-700/30">
                <div className="font-semibold text-gray-900 dark:text-gray-100">Total Physical</div>
                <div className="text-sm text-gray-600 dark:text-gray-400">{formatBytes(systemStatus.memory.totalPhysical)}</div>
              </div>
              <div className="glass-card p-3 rounded-lg bg-gray-50/50 dark:bg-gray-800/50 border border-gray-200/30 dark:border-gray-700/30">
                <div className="font-semibold text-gray-900 dark:text-gray-100">Available Physical</div>
                <div className="text-sm text-gray-600 dark:text-gray-400">{formatBytes(systemStatus.memory.availablePhysical)}</div>
              </div>
            </div>
            <div className="mt-4 glass-card p-4 rounded-lg bg-gray-50/50 dark:bg-gray-800/50 border border-gray-200/30 dark:border-gray-700/30">
              <div className="font-semibold text-gray-900 dark:text-gray-100">Memory Usage</div>
              <div className="text-sm text-gray-600 dark:text-gray-400 mb-2">{systemStatus.memory.usagePercentage?.toFixed(1) || '0'}%</div>
              <div className="w-full bg-gray-200 dark:bg-gray-700 rounded-full h-2">
                <div
                  className="h-2 rounded-full bg-gradient-to-r from-blue-500 to-blue-600 dark:from-blue-400 dark:to-blue-500 transition-all duration-300 shadow-sm"
                  style={{ width: `${Math.min(systemStatus.memory.usagePercentage || 0, 100)}%` }}
                />
              </div>
            </div>
          </CardContent>
        </Card>
      )}

      {/* Operating System */}
      {systemStatus.operatingSystem && (
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center">
              <Shield className="h-5 w-5 mr-2" />
              Operating System
            </CardTitle>
          </CardHeader>
          <CardContent>
            <div className="grid grid-cols-2 gap-4">
              <div className="glass-card p-3 rounded-lg bg-gray-50/50 dark:bg-gray-800/50 border border-gray-200/30 dark:border-gray-700/30">
                <div className="font-semibold text-gray-900 dark:text-gray-100">Name</div>
                <div className="text-sm text-gray-600 dark:text-gray-400">{systemStatus.operatingSystem.name || 'Unknown'}</div>
              </div>
              <div className="glass-card p-3 rounded-lg bg-gray-50/50 dark:bg-gray-800/50 border border-gray-200/30 dark:border-gray-700/30">
                <div className="font-semibold text-gray-900 dark:text-gray-100">Version</div>
                <div className="text-sm text-gray-600 dark:text-gray-400">{systemStatus.operatingSystem.version || 'Unknown'}</div>
              </div>
              <div className="glass-card p-3 rounded-lg bg-gray-50/50 dark:bg-gray-800/50 border border-gray-200/30 dark:border-gray-700/30">
                <div className="font-semibold text-gray-900 dark:text-gray-100">Architecture</div>
                <div className="text-sm text-gray-600 dark:text-gray-400">{systemStatus.operatingSystem.architecture || 'Unknown'}</div>
              </div>
              <div className="glass-card p-3 rounded-lg bg-gray-50/50 dark:bg-gray-800/50 border border-gray-200/30 dark:border-gray-700/30">
                <div className="font-semibold text-gray-900 dark:text-gray-100">Virtual Machine</div>
                <div className="text-sm text-gray-600 dark:text-gray-400">
                  <Badge variant={systemStatus.operatingSystem.isVirtualMachine ? "secondary" : "default"}>
                    {systemStatus.operatingSystem.isVirtualMachine ? (
                      <>
                        <AlertTriangle className="h-3 w-3 mr-1" />
                        Yes
                      </>
                    ) : (
                      <>
                        <CheckCircle className="h-3 w-3 mr-1" />
                        No
                      </>
                    )}
                  </Badge>
                </div>
              </div>
            </div>
          </CardContent>
        </Card>
      )}

      {/* Disk Information */}
      {systemStatus.disks && systemStatus.disks.length > 0 && (
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center">
              <Monitor className="h-5 w-5 mr-2" />
              Storage Devices
            </CardTitle>
          </CardHeader>
          <CardContent>
            <div className="space-y-3">
              {systemStatus.disks.map((disk, index) => (
                <div key={index} className="glass-card p-3 rounded-lg bg-gray-50/50 dark:bg-gray-800/50 border border-gray-200/30 dark:border-gray-700/30">
                  <div className="flex justify-between items-start">
                    <div>
                      <div className="font-semibold text-gray-900 dark:text-gray-100">{disk.name}</div>
                      <div className="text-sm text-gray-600 dark:text-gray-400">{disk.model}</div>
                    </div>
                    <div className="text-right">
                      <div className="font-semibold text-gray-900 dark:text-gray-100">{formatBytes(disk.size)}</div>
                      <div className="text-sm text-gray-600 dark:text-gray-400">Total Size</div>
                    </div>
                  </div>
                </div>
              ))}
            </div>
          </CardContent>
        </Card>
      )}

      {/* Network Interfaces */}
      {systemStatus.networkInterfaces && systemStatus.networkInterfaces.length > 0 && (
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center">
              <Network className="h-5 w-5 mr-2" />
              Network Interfaces
            </CardTitle>
          </CardHeader>
          <CardContent>
            <div className="space-y-3">
              {systemStatus.networkInterfaces.slice(0, 3).map((network, index) => (
                <div key={index} className="glass-card p-3 rounded-lg bg-gray-50/50 dark:bg-gray-800/50 border border-gray-200/30 dark:border-gray-700/30">
                  <div className="flex justify-between items-start">
                    <div>
                      <div className="font-semibold text-gray-900 dark:text-gray-100">{network.displayName || network.name}</div>
                      <div className="text-sm text-gray-600 dark:text-gray-400">
                        {network.ipv4Addresses && network.ipv4Addresses.length > 0 ? network.ipv4Addresses[0] : 'No IP'}
                      </div>
                    </div>
                    <div className="text-right">
                      <Badge variant={network.isUp ? "default" : "secondary"}>
                        {network.isUp ? (
                          <>
                            <CheckCircle className="h-3 w-3 mr-1" />
                            Active
                          </>
                        ) : (
                          <>
                            <XCircle className="h-3 w-3 mr-1" />
                            Inactive
                          </>
                        )}
                      </Badge>
                    </div>
                  </div>
                </div>
              ))}
            </div>
          </CardContent>
        </Card>
      )}
    </div>
  );
};

export default SystemInfoPage;