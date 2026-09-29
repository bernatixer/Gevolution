import ChickenEntity from './chicken';
import * as NEAT from 'neataptic';

var network = new NEAT.architect.Perceptron(2, 2, 1);

var trainingSet = [
  { input: [0,-1], output: [0] },
  { input: [0,-1.5], output: [0] },
  { input: [0,-2.5], output: [0] },
  { input: [0,0], output: [0] },
  { input: [0,0.1], output: [0] },
  { input: [0,0.2], output: [0] },
  { input: [0,0.3], output: [0] },
  { input: [0,0.4], output: [0] },
  { input: [0,0.5], output: [0] },
  { input: [0,1.1], output: [0] },
  { input: [0,1.2], output: [0] },
  { input: [0,1.3], output: [0] },
  { input: [0,1.4], output: [0] },
  { input: [0,1.5], output: [0] },
  { input: [1,0], output: [1] },
  { input: [1,0.1], output: [1] },
  { input: [1,0.11], output: [1] },

  { input: [0,1], output: [0] },
  { input: [1,1], output: [0] },

  { input: [0,2], output: [0] },

  { input: [0,3], output: [0] },
  { input: [0,4], output: [0] },
  { input: [0,5], output: [0] },
];
network.train(trainingSet, {
  error: 0.01
});


export default class EntitiesManager {
    constructor(terrain, scene, physics) {
        this.entities = []
        this.terrain = terrain;
        this.scene = scene;
        this.physics = physics;
    }

    addChicken() {
        const chicken = new ChickenEntity(this.terrain, this.scene, this.physics);
        this.entities.push(chicken);
    }

    applyAction() {
        for (let i=0; i<this.entities.length; ++i) {
            if (this.entities[i].ready) {
                this.jump(this.entities[i]);
            }
        }
    }

    jump(entity) {
        const velocity = entity["object"].body.velocity.y;
        const jump = network.activate([entity["isColliding"] ? 1 : 0, velocity])[0];
        if (jump > 0.90) entity["object"].body.applyForceY(20);
    }

}
