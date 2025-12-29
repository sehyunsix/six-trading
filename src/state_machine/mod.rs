use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SystemState {
    Booting,
    Accumulating, // Waiting for enough data
    Analyzing,    // Checking stability
    Trading,      // Active
    Cooldown,     // Paused
}

impl SystemState {
    pub fn to_index(self) -> usize {
        match self {
            SystemState::Booting => 0,
            SystemState::Accumulating => 1,
            SystemState::Analyzing => 2,
            SystemState::Trading => 3,
            SystemState::Cooldown => 4,
        }
    }

    pub fn all() -> Vec<SystemState> {
        vec![
            SystemState::Booting,
            SystemState::Accumulating,
            SystemState::Analyzing,
            SystemState::Trading,
            SystemState::Cooldown,
        ]
    }
}

impl Default for SystemState {
    fn default() -> Self {
        Self::Booting
    }
}

pub struct StateMachine {
    current_state: SystemState,
    last_transition_time: std::time::Instant,
    // [FromState][ToState] counter
    transition_matrix: [[u64; 5]; 5],
    // Predictive probabilities based on real-time scoring
    inferred_matrix: [[f64; 5]; 5],
}

impl StateMachine {
    pub fn new() -> Self {
        Self {
            current_state: SystemState::Booting,
            last_transition_time: std::time::Instant::now(),
            transition_matrix: [[0; 5]; 5],
            inferred_matrix: [[0.0; 5]; 5],
        }
    }

    pub fn get_state(&self) -> SystemState {
        self.current_state
    }

    pub fn transition_to(&mut self, new_state: SystemState) {
        if self.current_state != new_state {
            log::info!("State Transition: {:?} -> {:?}", self.current_state, new_state);
            
            // Record transition
            let from_idx = self.current_state.to_index();
            let to_idx = new_state.to_index();
            self.transition_matrix[from_idx][to_idx] += 1;

            self.current_state = new_state;
            self.last_transition_time = std::time::Instant::now();
        }
    }

    pub fn get_transition_probabilities(&self) -> Vec<Vec<f64>> {
        let mut probs = vec![vec![0.0; 5]; 5];
        for i in 0..5 {
            let row_total: u64 = self.transition_matrix[i].iter().sum();
            if row_total > 0 {
                for j in 0..5 {
                    probs[i][j] = (self.transition_matrix[i][j] as f64) / (row_total as f64);
                }
            }
        }
        probs
    }

    pub fn get_inferred_probabilities(&self) -> Vec<Vec<f64>> {
        self.inferred_matrix.iter().map(|row| row.to_vec()).collect()
    }

    /// Update inferred probabilities based on real-time market scores
    pub fn update_inferred_probabilities(&mut self, spread_score: f64, imbalance_score: f64, volatility_score: f64) {
        let current_idx = self.current_state.to_index();
        
        // Reset row for current state in inferred matrix
        let mut new_probs = [0.0; 5];
        
        // Logical inference (simplified for demonstration)
        match self.current_state {
            SystemState::Booting | SystemState::Accumulating => {
                // If spread is tight and imbalance exists, likely moving to Analyzing
                new_probs[SystemState::Analyzing.to_index()] = (1.0 - spread_score).max(0.1);
                new_probs[SystemState::Accumulating.to_index()] = spread_score.max(0.1);
            }
            SystemState::Analyzing => {
                // High imbalance increases chance of Trading
                new_probs[SystemState::Trading.to_index()] = imbalance_score.abs().min(0.9);
                new_probs[SystemState::Analyzing.to_index()] = (1.0 - imbalance_score.abs()).max(0.1);
                
                // High volatility pushes back to Cooldown or Analyzing
                if volatility_score > 0.7 {
                    new_probs[SystemState::Cooldown.to_index()] = volatility_score;
                }
            }
            SystemState::Trading => {
                // High volatility in Trading might trigger Cooldown
                new_probs[SystemState::Cooldown.to_index()] = volatility_score.max(0.1);
                new_probs[SystemState::Trading.to_index()] = (1.0 - volatility_score).max(0.1);
            }
            SystemState::Cooldown => {
                // Cooldown eventually moves back to Analyzing when volatility drops
                new_probs[SystemState::Analyzing.to_index()] = (1.0 - volatility_score).max(0.1);
                new_probs[SystemState::Cooldown.to_index()] = volatility_score.max(0.1);
            }
        }

        // Normalize the row
        let sum: f64 = new_probs.iter().sum();
        if sum > 0.0 {
            for j in 0..5 {
                self.inferred_matrix[current_idx][j] = new_probs[j] / sum;
            }
        }
    }

    pub fn is_stable(&self) -> bool {
        match self.current_state {
            SystemState::Accumulating => self.last_transition_time.elapsed().as_secs() > 5,
            _ => true,
        }
    }
}
